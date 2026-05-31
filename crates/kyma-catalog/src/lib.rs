//! Postgres-backed catalog implementation.
//!
//! Implements [`kyma_core::catalog::Catalog`]. The metadata model mirrors
//! Apache Iceberg's hierarchy (table → snapshot → manifest → data_file).
//!
//! # Commit flow
//!
//! 1. [`PostgresCatalog::begin_snapshot`] returns a `PgSnapshotTxn` holding
//!    the parent snapshot id in memory. No pg tx is held open.
//! 2. Caller appends extents via `add_extent` / `remove_extents`. Again, no
//!    pg tx yet — accumulation is pure in-memory.
//! 3. [`PgSnapshotTxn::commit`] runs **one short pg tx** that:
//!    a. Inserts the new snapshot row and manifests + extents.
//!    b. CASes `tables.current_snapshot_id` against the parent id.
//!    c. On CAS mismatch, returns `CatalogError::Conflict` — caller retries.

#![forbid(unsafe_code)]

mod error;
pub mod saved_views;
mod snapshot;

pub use snapshot::PgSnapshotTxn;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use kyma_core::catalog::{
    BackgroundTask, Catalog, CleanupResult, ColumnInfo, ColumnPrune, Dashboard, DashboardPanel,
    DashboardPanelInput, DashboardUpdate, DashboardWithPanels, ExtentManifest, GraphRegistration,
    GraphSpec, IngestLedgerEntry, NodeInfo, NodeLease, NodeRole, PrunePredicate, RefreshClaim,
    SnapshotTxn, TableConfig, TableRef, TokenPrincipal, User,
};
use kyma_core::errors::{CatalogError, Result};
use kyma_core::tenant::TenantId;
use kyma_core::types::{DatabaseId, ExtentId, NodeId, SchemaSnapshotId, SnapshotId, TableId};
use serde_json::Value as Json;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// The Postgres-backed catalog.
///
/// Holds a connection pool; cloneable and thread-safe.
#[derive(Debug, Clone)]
pub struct PostgresCatalog {
    pool: PgPool,
}

impl PostgresCatalog {
    /// Connect to Postgres and run migrations.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .acquire_timeout(Duration::from_secs(10))
            .connect(database_url)
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| CatalogError::Sql(format!("migration failed: {e}")))?;

        Ok(Self { pool })
    }

    /// Borrow the underlying pool (useful for tests and the kafka_offsets tx).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl Catalog for PostgresCatalog {
    fn as_ref_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }

    async fn create_database_in_tenant(
        &self,
        tenant: TenantId,
        name: &str,
    ) -> Result<DatabaseId> {
        let row: (Uuid,) = sqlx::query_as(
            "INSERT INTO databases (tenant_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(DatabaseId::from_uuid(row.0))
    }

    async fn create_table_in_tenant(
        &self,
        tenant: TenantId,
        database_id: DatabaseId,
        name: &str,
        schema: Arc<arrow_schema::Schema>,
        config: TableConfig,
    ) -> Result<TableId> {
        let schema_json = schema_to_json(&schema)?;
        let config_json =
            serde_json::to_value(&config).map_err(|e| CatalogError::Sql(e.to_string()))?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        // 0. Verify the database belongs to this tenant. Reject mismatches
        //    so a caller can never create a table under a database it
        //    doesn't own (cross-tenant link prevention).
        let db_tenant: Option<(Uuid,)> =
            sqlx::query_as("SELECT tenant_id FROM databases WHERE id = $1")
                .bind(database_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| CatalogError::Sql(e.to_string()))?;
        let db_tenant = db_tenant.ok_or_else(|| {
            CatalogError::Sql(format!("database {} not found", database_id))
        })?;
        if db_tenant.0 != tenant.as_uuid() {
            return Err(CatalogError::Sql(format!(
                "database {} does not belong to tenant {}",
                database_id, tenant
            ))
            .into());
        }

        // 1. Insert the table with NULL snapshot pointers (bootstrap).
        let (table_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO tables (tenant_id, database_id, name, config)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(database_id.as_uuid())
        .bind(name)
        .bind(&config_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        // 2. Insert the initial schema snapshot.
        let (schema_snap_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO schema_snapshots (tenant_id, table_id, arrow_schema)
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(table_id)
        .bind(&schema_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        // 3. Insert snapshot #0 (empty).
        let (snap_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO snapshots (tenant_id, table_id, parent_id, sequence_number, schema_snapshot_id, summary)
             VALUES ($1, $2, NULL, 0, $3, $4) RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(table_id)
        .bind(schema_snap_id)
        .bind(serde_json::json!({ "operation": "bootstrap" }))
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        // 4. Point the table at the initial snapshot + schema.
        sqlx::query(
            "UPDATE tables SET current_snapshot_id = $1, schema_snapshot_id = $2 WHERE id = $3",
        )
        .bind(snap_id)
        .bind(schema_snap_id)
        .bind(table_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        Ok(TableId::from_uuid(table_id))
    }

    async fn lookup_table_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
    ) -> Result<TableRef> {
        let row = sqlx::query(
            "SELECT t.id, t.database_id, t.current_snapshot_id, t.schema_snapshot_id,
                    ss.arrow_schema, t.config
             FROM tables t
             JOIN databases d ON d.id = t.database_id AND d.tenant_id = $1
             LEFT JOIN schema_snapshots ss ON ss.id = t.schema_snapshot_id
             WHERE t.tenant_id = $1 AND d.name = $2 AND t.name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?
        .ok_or_else(|| CatalogError::TableNotFound {
            database: database.to_owned(),
            name: name.to_owned(),
        })?;

        let id: Uuid = row.try_get("id").map_err(sql_err)?;
        let database_id: Uuid = row.try_get("database_id").map_err(sql_err)?;
        let current_snapshot_id: Option<Uuid> =
            row.try_get("current_snapshot_id").map_err(sql_err)?;
        let schema_snapshot_id: Option<Uuid> =
            row.try_get("schema_snapshot_id").map_err(sql_err)?;
        let schema_json: Json = row.try_get("arrow_schema").map_err(sql_err)?;
        let config_json: Json = row.try_get("config").map_err(sql_err)?;

        let schema = json_to_schema(&schema_json)?;
        let config: TableConfig = serde_json::from_value(config_json).unwrap_or_default();
        let current_snapshot_id = current_snapshot_id.ok_or_else(|| {
            CatalogError::Sql("table has no current snapshot (bootstrap row)".into())
        })?;
        let schema_snapshot_id = schema_snapshot_id
            .ok_or_else(|| CatalogError::Sql("table has no schema snapshot".into()))?;

        Ok(TableRef {
            id: TableId::from_uuid(id),
            database_id: DatabaseId::from_uuid(database_id),
            name: name.to_owned(),
            current_snapshot_id: SnapshotId::from_uuid(current_snapshot_id),
            schema_snapshot_id: SchemaSnapshotId::from_uuid(schema_snapshot_id),
            schema,
            config,
        })
    }

    async fn list_tables_in_database_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
    ) -> Result<Vec<TableRef>> {
        let rows = sqlx::query(
            "SELECT t.id, t.database_id, t.name, t.current_snapshot_id, t.schema_snapshot_id,
                    ss.arrow_schema, t.config
             FROM tables t
             JOIN databases d ON d.id = t.database_id AND d.tenant_id = $1
             LEFT JOIN schema_snapshots ss ON ss.id = t.schema_snapshot_id
             WHERE t.tenant_id = $1 AND d.name = $2
             ORDER BY t.name",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id: Uuid = row.try_get("id").map_err(sql_err)?;
            let database_id: Uuid = row.try_get("database_id").map_err(sql_err)?;
            let name: String = row.try_get("name").map_err(sql_err)?;
            let current_snapshot_id: Option<Uuid> =
                row.try_get("current_snapshot_id").map_err(sql_err)?;
            let schema_snapshot_id: Option<Uuid> =
                row.try_get("schema_snapshot_id").map_err(sql_err)?;
            let schema_json: Option<Json> = row.try_get("arrow_schema").ok();
            let config_json: Json = row.try_get("config").map_err(sql_err)?;

            // Skip rows where schema not yet committed (bootstrap glitch window).
            let (Some(schema_json), Some(current_snapshot_id), Some(schema_snapshot_id)) =
                (schema_json, current_snapshot_id, schema_snapshot_id)
            else {
                continue;
            };
            let schema = json_to_schema(&schema_json)?;
            let config: TableConfig = serde_json::from_value(config_json).unwrap_or_default();
            out.push(TableRef {
                id: TableId::from_uuid(id),
                database_id: DatabaseId::from_uuid(database_id),
                name,
                current_snapshot_id: SnapshotId::from_uuid(current_snapshot_id),
                schema_snapshot_id: SchemaSnapshotId::from_uuid(schema_snapshot_id),
                schema,
                config,
            });
        }
        Ok(out)
    }

    async fn alter_table_add_column(
        &self,
        table_id: TableId,
        column_name: &str,
        column_type: &str,
    ) -> Result<SchemaSnapshotId> {
        // Basic validation of column type — reject garbage early.
        let _ = string_to_arrow_type(column_type)?;

        // Fetch the current schema snapshot jsonb, append the new column,
        // insert a new schema_snapshot row, CAS on schema_snapshot_id.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let row = sqlx::query(
            "SELECT t.tenant_id, t.schema_snapshot_id, ss.arrow_schema
             FROM tables t
             JOIN schema_snapshots ss ON ss.id = t.schema_snapshot_id
             WHERE t.id = $1
             FOR UPDATE",
        )
        .bind(table_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql_err)?
        .ok_or_else(|| CatalogError::Sql(format!("table {table_id} not found")))?;

        let tenant_id: Uuid = row.try_get("tenant_id").map_err(sql_err)?;
        let current_schema_snapshot_id: Uuid =
            row.try_get("schema_snapshot_id").map_err(sql_err)?;
        let mut schema_json: Json = row.try_get("arrow_schema").map_err(sql_err)?;

        // Reject duplicate column names.
        if let Some(fields) = schema_json.get("fields").and_then(|v| v.as_array()) {
            if fields
                .iter()
                .any(|f| f.get("name").and_then(|n| n.as_str()) == Some(column_name))
            {
                return Err(
                    CatalogError::Sql(format!("column {column_name} already exists")).into(),
                );
            }
        }

        // Append the new (nullable) column.
        if let Some(arr) = schema_json.get_mut("fields").and_then(|v| v.as_array_mut()) {
            arr.push(serde_json::json!({
                "name":     column_name,
                "type":     column_type,
                "nullable": true,
            }));
        } else {
            return Err(CatalogError::Sql("existing schema has no `fields` array".into()).into());
        }

        // Insert the new schema_snapshot, carrying the table's tenant_id
        // (NOT NULL since migration 007 — issue #18).
        let (new_schema_snap_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO schema_snapshots (tenant_id, table_id, arrow_schema)
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(tenant_id)
        .bind(table_id.as_uuid())
        .bind(&schema_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_err)?;

        // CAS on the tables row: only advance if the current schema
        // snapshot is still the one we read.
        let swapped = sqlx::query(
            "UPDATE tables SET schema_snapshot_id = $1
             WHERE id = $2 AND schema_snapshot_id = $3",
        )
        .bind(new_schema_snap_id)
        .bind(table_id.as_uuid())
        .bind(current_schema_snapshot_id)
        .execute(&mut *tx)
        .await
        .map_err(sql_err)?;
        if swapped.rows_affected() != 1 {
            return Err(CatalogError::Conflict.into());
        }

        tx.commit()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        Ok(SchemaSnapshotId::from_uuid(new_schema_snap_id))
    }

    async fn begin_snapshot_in_tenant(
        &self,
        tenant: TenantId,
        table_id: TableId,
    ) -> Result<Box<dyn SnapshotTxn>> {
        let row = sqlx::query(
            "SELECT current_snapshot_id, schema_snapshot_id
             FROM tables
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(table_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?
        .ok_or_else(|| {
            CatalogError::Sql(format!(
                "table {table_id} not found in tenant {tenant} for begin_snapshot"
            ))
        })?;

        let parent: Uuid = row
            .try_get::<Option<Uuid>, _>("current_snapshot_id")
            .map_err(sql_err)?
            .ok_or_else(|| CatalogError::Sql("table has no current_snapshot_id".into()))?;
        let schema_snap: Uuid = row
            .try_get::<Option<Uuid>, _>("schema_snapshot_id")
            .map_err(sql_err)?
            .ok_or_else(|| CatalogError::Sql("table has no schema_snapshot_id".into()))?;

        Ok(Box::new(PgSnapshotTxn::new(
            self.pool.clone(),
            tenant,
            table_id,
            SnapshotId::from_uuid(parent),
            SchemaSnapshotId::from_uuid(schema_snap),
        )))
    }

    async fn list_extents_in_tenant(
        &self,
        tenant: TenantId,
        table_id: TableId,
        _snapshot: SnapshotId,
        prune: &PrunePredicate,
    ) -> Result<Vec<ExtentManifest>> {
        // Honors `time_range`, `required_paths`, and `column_predicates`.
        // Column predicates use the equality-index stored in each extent's
        // `column_stats->{col}->>'distinct'`:
        //   - `distinct` is NULL → column not indexable or cardinality
        //     exceeded threshold; extent *must* be read (can't prune).
        //   - `distinct` is an array → extent only holds those values.
        //     Skip the extent unless one of the target values is in the array.

        let mut sql = String::from(
            "SELECT id, table_id, schema_snapshot_id, object_path, byte_size, row_count,
                    min_timestamp, max_timestamp, column_stats, present_paths,
                    compaction_gen, created_at
             FROM extents
             WHERE tenant_id = $1 AND table_id = $2 AND deleted_at IS NULL",
        );
        let mut arg_index = 3;
        if prune.time_range.is_some() {
            sql.push_str(&format!(
                " AND tstzrange(min_timestamp, max_timestamp) && tstzrange(${arg_index}, ${})",
                arg_index + 1
            ));
            arg_index += 2;
        }
        if !prune.required_paths.is_empty() {
            sql.push_str(&format!(" AND present_paths @> ${arg_index}"));
            arg_index += 1;
        }

        // Column predicates: one JSONB check per column. Postgres can't
        // parametrize a JSON key path, so the column name must be inlined —
        // we SQL-escape it here. The values are parametrized normally.
        let mut predicate_binds: Vec<serde_json::Value> = Vec::new();
        for (col_name, pred) in &prune.column_predicates {
            let safe_col = col_name.replace('\'', "''"); // reject SQL-injection via column names
            let col_ref = format!("column_stats->'{safe_col}'->'distinct'");
            match pred {
                ColumnPrune::Equals(v) => {
                    // Include the extent if its distinct set is unknown
                    // (null) OR contains the target value.
                    sql.push_str(&format!(
                        " AND ({col_ref} IS NULL OR {col_ref} = 'null'::jsonb
                                         OR {col_ref} @> ${arg_index})"
                    ));
                    predicate_binds.push(serde_json::json!([v]));
                    arg_index += 1;
                }
                ColumnPrune::InSet(vs) => {
                    // Any overlap between the target set and the distinct set
                    // means the extent *might* contain a matching row.
                    // `?|` is "any of these keys exists" but we need a value-
                    // overlap op; use jsonb path existence via subquery.
                    // Equivalent: the intersection is non-empty.
                    sql.push_str(&format!(
                        " AND ({col_ref} IS NULL OR {col_ref} = 'null'::jsonb
                                         OR EXISTS (
                                             SELECT 1
                                             FROM jsonb_array_elements({col_ref}) d(x)
                                             WHERE x IN (SELECT jsonb_array_elements(${arg_index}))
                                         ))"
                    ));
                    predicate_binds.push(serde_json::json!(vs));
                    arg_index += 1;
                }
                ColumnPrune::Between { low, high } => {
                    // For numeric ranges we'd want per-extent min/max; the
                    // distinct set still lets us handle this correctly (if
                    // known), by checking whether any element is in range.
                    sql.push_str(&format!(
                        " AND ({col_ref} IS NULL OR {col_ref} = 'null'::jsonb
                                         OR EXISTS (
                                             SELECT 1
                                             FROM jsonb_array_elements({col_ref}) d(x)
                                             WHERE x >= ${arg_index} AND x <= ${}
                                         ))",
                        arg_index + 1
                    ));
                    predicate_binds.push(low.clone());
                    predicate_binds.push(high.clone());
                    arg_index += 2;
                }
                ColumnPrune::ContainsTokens(tokens) => {
                    // Text-search pruning: check the column's token set
                    // contains *all* of the query's tokens. `@>` on JSONB
                    // arrays does "left contains right as a subset," which
                    // is exactly what we want.
                    let tok_ref = format!("column_stats->'{safe_col}'->'tokens'");
                    sql.push_str(&format!(
                        " AND ({tok_ref} IS NULL OR {tok_ref} = 'null'::jsonb
                                         OR {tok_ref} @> ${arg_index})"
                    ));
                    predicate_binds.push(serde_json::json!(tokens));
                    arg_index += 1;
                }
            }
        }
        #[allow(unused_assignments)]
        {
            arg_index = arg_index; // silence last-increment warning
        }
        sql.push_str(" ORDER BY min_timestamp DESC NULLS LAST");

        let mut q = sqlx::query(&sql)
            .bind(tenant.as_uuid())
            .bind(table_id.as_uuid());
        if let Some(tr) = &prune.time_range {
            q = q.bind(tr.start_inclusive).bind(tr.end_exclusive);
        }
        if !prune.required_paths.is_empty() {
            q = q.bind(&prune.required_paths);
        }
        for v in &predicate_binds {
            q = q.bind(v);
        }

        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(ExtentManifest {
                id: ExtentId::from_uuid(row.try_get("id").map_err(sql_err)?),
                table_id: TableId::from_uuid(row.try_get("table_id").map_err(sql_err)?),
                schema_snapshot_id: SchemaSnapshotId::from_uuid(
                    row.try_get("schema_snapshot_id").map_err(sql_err)?,
                ),
                object_path: row.try_get("object_path").map_err(sql_err)?,
                byte_size: row.try_get::<i64, _>("byte_size").map_err(sql_err)? as u64,
                row_count: row.try_get::<i64, _>("row_count").map_err(sql_err)? as u64,
                min_timestamp: row.try_get("min_timestamp").map_err(sql_err)?,
                max_timestamp: row.try_get("max_timestamp").map_err(sql_err)?,
                column_stats: row.try_get("column_stats").map_err(sql_err)?,
                present_paths: row.try_get("present_paths").map_err(sql_err)?,
                compaction_gen: row.try_get::<i32, _>("compaction_gen").map_err(sql_err)? as u32,
                created_at: row.try_get("created_at").map_err(sql_err)?,
            });
        }
        Ok(out)
    }

    async fn gc_candidates(&self, before: DateTime<Utc>) -> Result<Vec<ExtentId>> {
        let rows: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM extents WHERE deleted_at IS NOT NULL AND deleted_at < $1",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(rows
            .into_iter()
            .map(|(u,)| ExtentId::from_uuid(u))
            .collect())
    }

    async fn delete_extent_rows(&self, extents: &[ExtentId]) -> Result<()> {
        if extents.is_empty() {
            return Ok(());
        }
        let ids: Vec<Uuid> = extents.iter().map(|e| *e.as_uuid()).collect();
        sqlx::query("DELETE FROM extents WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;
        Ok(())
    }

    async fn cleanup_soft_deleted_extents_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        table: &str,
        before: DateTime<Utc>,
    ) -> Result<CleanupResult> {
        // 1. Resolve (tenant, database, table) → table_id. Reuses the
        //    tenant-aware lookup so a caller in tenant A can never cleanup
        //    extents under tenant B's table even if it knows the name.
        let table_ref = self.lookup_table_in_tenant(tenant, database, table).await?;
        let table_uuid = *table_ref.id.as_uuid();

        // 2. Collect aggregate stats before deletion so we can return them.
        //    CAST to bigint: SUM over a bigint column returns NUMERIC in
        //    Postgres, which doesn't decode directly to i64 via sqlx.
        let agg_row = sqlx::query(
            "SELECT COUNT(*) AS cnt,
                    CAST(COALESCE(SUM(row_count), 0) AS bigint) AS rows_freed,
                    CAST(COALESCE(SUM(byte_size), 0) AS bigint) AS bytes_freed
             FROM extents
             WHERE tenant_id = $1
               AND table_id = $2
               AND deleted_at IS NOT NULL
               AND deleted_at < $3",
        )
        .bind(tenant.as_uuid())
        .bind(table_uuid)
        .bind(before)
        .fetch_one(&self.pool)
        .await
        .map_err(sql_err)?;

        let extents_deleted: i64 = agg_row.try_get("cnt").map_err(sql_err)?;
        let rows_freed: i64 = agg_row.try_get("rows_freed").map_err(sql_err)?;
        let bytes_freed: i64 = agg_row.try_get("bytes_freed").map_err(sql_err)?;

        // 3. Hard-delete the qualifying extent rows.
        if extents_deleted > 0 {
            sqlx::query(
                "DELETE FROM extents
                 WHERE tenant_id = $1
                   AND table_id = $2
                   AND deleted_at IS NOT NULL
                   AND deleted_at < $3",
            )
            .bind(tenant.as_uuid())
            .bind(table_uuid)
            .bind(before)
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;
        }

        Ok(CleanupResult {
            extents_deleted: extents_deleted as u64,
            rows_freed: rows_freed as u64,
            bytes_freed: bytes_freed as u64,
        })
    }

    async fn register_node(&self, info: NodeInfo) -> Result<NodeLease> {
        let lease_id = Uuid::new_v4();
        let role_str = role_to_str(info.role);
        let (node_id, last_heartbeat): (Uuid, DateTime<Utc>) = sqlx::query_as(
            "INSERT INTO nodes (role, endpoint, capabilities, lease_id)
             VALUES ($1, $2, $3, $4)
             RETURNING id, last_heartbeat",
        )
        .bind(role_str)
        .bind(&info.endpoint)
        .bind(&info.capabilities)
        .bind(lease_id)
        .fetch_one(&self.pool)
        .await
        .map_err(sql_err)?;

        // Lease expiration is a derived concept for now: 3× heartbeat interval
        // beyond the last heartbeat. The actual TTL policy lives in the
        // heartbeat-aware code paths.
        Ok(NodeLease {
            node_id: NodeId::from_uuid(node_id),
            lease_id,
            expires_at: last_heartbeat + chrono::Duration::seconds(60),
        })
    }

    async fn heartbeat(&self, lease: &NodeLease) -> Result<()> {
        let updated = sqlx::query(
            "UPDATE nodes SET last_heartbeat = now()
             WHERE id = $1 AND lease_id = $2",
        )
        .bind(lease.node_id.as_uuid())
        .bind(lease.lease_id)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;
        if updated.rows_affected() == 0 {
            return Err(CatalogError::Sql(format!(
                "heartbeat rejected — node {} / lease {} unknown (lease stolen or deregistered)",
                lease.node_id, lease.lease_id
            ))
            .into());
        }
        Ok(())
    }

    async fn deregister_node(&self, lease: NodeLease) -> Result<()> {
        sqlx::query("DELETE FROM nodes WHERE id = $1 AND lease_id = $2")
            .bind(lease.node_id.as_uuid())
            .bind(lease.lease_id)
            .execute(&self.pool)
            .await
            .map_err(sql_err)?;
        Ok(())
    }

    async fn list_live_nodes(
        &self,
        max_stale_secs: u32,
    ) -> Result<Vec<kyma_core::catalog::LiveNode>> {
        let rows: Vec<(Uuid, String, String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT id, role, endpoint, last_heartbeat FROM nodes
             WHERE last_heartbeat > now() - make_interval(secs => $1)
             ORDER BY id",
        )
        .bind(max_stale_secs as f64)
        .fetch_all(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, role, endpoint, hb)| kyma_core::catalog::LiveNode {
                node_id: NodeId::from_uuid(id),
                role: str_to_role(&role),
                endpoint,
                last_heartbeat: hb,
            })
            .collect())
    }

    async fn submit_task(
        &self,
        kind: &str,
        table_id: Option<kyma_core::types::TableId>,
        payload: serde_json::Value,
        priority: i32,
    ) -> Result<Uuid> {
        let (id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO background_tasks (kind, table_id, payload, priority)
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(kind)
        .bind(table_id.map(|t| *t.as_uuid()))
        .bind(&payload)
        .bind(priority)
        .fetch_one(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(id)
    }

    async fn claim_task(
        &self,
        kind: &str,
        node_id: NodeId,
        lease: chrono::Duration,
    ) -> Result<Option<BackgroundTask>> {
        // Single-statement atomic claim: CTE picks the next pending task,
        // main UPDATE flips it to `claimed` with our node + lease expiry.
        // FOR UPDATE SKIP LOCKED lets many workers run concurrently without
        // stepping on each other.
        let row = sqlx::query(
            "WITH next AS (
                SELECT id
                FROM background_tasks
                WHERE kind = $1
                  AND (status = 'pending'
                       OR (status = 'claimed' AND claim_expires_at < now()))
                ORDER BY priority DESC, created_at
                LIMIT 1
                FOR UPDATE SKIP LOCKED
             )
             UPDATE background_tasks t
             SET status = 'claimed',
                 claimed_by = $2,
                 claim_expires_at = now() + ($3 || ' seconds')::interval,
                 attempt = t.attempt + 1,
                 updated_at = now()
             FROM next
             WHERE t.id = next.id
             RETURNING t.id, t.kind, t.table_id, t.payload, t.priority, t.attempt,
                       t.max_attempts, t.claim_expires_at",
        )
        .bind(kind)
        .bind(node_id.as_uuid())
        .bind(lease.num_seconds().to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?;
        let Some(row) = row else { return Ok(None) };

        let table_id: Option<Uuid> = row.try_get("table_id").map_err(sql_err)?;
        Ok(Some(BackgroundTask {
            id: row.try_get("id").map_err(sql_err)?,
            kind: row.try_get("kind").map_err(sql_err)?,
            table_id: table_id.map(kyma_core::types::TableId::from_uuid),
            payload: row.try_get("payload").map_err(sql_err)?,
            priority: row.try_get("priority").map_err(sql_err)?,
            attempt: row.try_get("attempt").map_err(sql_err)?,
            max_attempts: row.try_get("max_attempts").map_err(sql_err)?,
            claim_expires_at: row.try_get("claim_expires_at").map_err(sql_err)?,
        }))
    }

    async fn complete_task(&self, task_id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE background_tasks
             SET status = 'done', updated_at = now(), claim_expires_at = NULL
             WHERE id = $1 AND status = 'claimed'",
        )
        .bind(task_id)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(())
    }

    async fn fail_task(&self, task_id: Uuid, error: &str) -> Result<()> {
        // Reset to pending if attempts remain; else mark failed.
        sqlx::query(
            "UPDATE background_tasks
             SET status = CASE
                            WHEN attempt >= max_attempts THEN 'failed'
                            ELSE 'pending'
                          END,
                 claim_expires_at = NULL,
                 claimed_by = NULL,
                 last_error = $2,
                 updated_at = now()
             WHERE id = $1",
        )
        .bind(task_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(sql_err)?;
        Ok(())
    }

    async fn lookup_idempotency_in_tenant(
        &self,
        tenant: TenantId,
        key: &str,
    ) -> Result<Option<IngestLedgerEntry>> {
        let row = sqlx::query(
            "SELECT table_id, snapshot_id, rows_ingested, bytes_written, applied_at
             FROM ingest_ledger
             WHERE tenant_id = $1 AND idempotency_key = $2 AND ttl_expires_at > now()",
        )
        .bind(tenant.as_uuid())
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(IngestLedgerEntry {
            table_id: TableId::from_uuid(row.try_get("table_id").map_err(sql_err)?),
            snapshot_id: SnapshotId::from_uuid(row.try_get("snapshot_id").map_err(sql_err)?),
            rows_ingested: row.try_get::<i64, _>("rows_ingested").map_err(sql_err)? as u64,
            bytes_written: row.try_get::<i64, _>("bytes_written").map_err(sql_err)? as u64,
            applied_at: row.try_get("applied_at").map_err(sql_err)?,
        }))
    }

    async fn record_idempotency_in_tenant(
        &self,
        tenant: TenantId,
        key: &str,
        entry: IngestLedgerEntry,
        ttl: chrono::Duration,
    ) -> Result<Option<IngestLedgerEntry>> {
        let expires_at = entry.applied_at + ttl;
        let row = sqlx::query(
            "INSERT INTO ingest_ledger
                (tenant_id, idempotency_key, table_id, snapshot_id, rows_ingested, bytes_written,
                 applied_at, ttl_expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (tenant_id, idempotency_key) DO NOTHING
             RETURNING table_id, snapshot_id, rows_ingested, bytes_written, applied_at",
        )
        .bind(tenant.as_uuid())
        .bind(key)
        .bind(entry.table_id.as_uuid())
        .bind(entry.snapshot_id.as_uuid())
        .bind(entry.rows_ingested as i64)
        .bind(entry.bytes_written as i64)
        .bind(entry.applied_at)
        .bind(expires_at)
        .fetch_optional(&self.pool)
        .await
        .map_err(sql_err)?;
        let Some(row) = row else { return Ok(None) };
        Ok(Some(IngestLedgerEntry {
            table_id: TableId::from_uuid(row.try_get("table_id").map_err(sql_err)?),
            snapshot_id: SnapshotId::from_uuid(row.try_get("snapshot_id").map_err(sql_err)?),
            rows_ingested: row.try_get::<i64, _>("rows_ingested").map_err(sql_err)? as u64,
            bytes_written: row.try_get::<i64, _>("bytes_written").map_err(sql_err)? as u64,
            applied_at: row.try_get("applied_at").map_err(sql_err)?,
        }))
    }

    // --- dashboards ---

    async fn create_dashboard_in_tenant(
        &self,
        tenant: TenantId,
        name: &str,
        description: Option<&str>,
    ) -> std::result::Result<Dashboard, CatalogError> {
        let row = sqlx::query(
            "INSERT INTO dashboards (tenant_id, name, description)
             VALUES ($1, $2, $3)
             RETURNING id, name, description, time_range_preset,
                       refresh_interval_seconds, created_at, updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(name)
        .bind(description)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        row_to_dashboard(&row)
    }

    async fn list_dashboards_in_tenant(
        &self,
        tenant: TenantId,
    ) -> std::result::Result<Vec<Dashboard>, CatalogError> {
        let rows = sqlx::query(
            "SELECT id, name, description, time_range_preset,
                    refresh_interval_seconds, created_at, updated_at
             FROM dashboards
             WHERE tenant_id = $1
             ORDER BY updated_at DESC",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        rows.iter().map(row_to_dashboard).collect()
    }

    async fn get_dashboard_in_tenant(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> std::result::Result<Option<DashboardWithPanels>, CatalogError> {
        let maybe_row = sqlx::query(
            "SELECT id, name, description, time_range_preset,
                    refresh_interval_seconds, created_at, updated_at
             FROM dashboards
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let Some(row) = maybe_row else {
            return Ok(None);
        };
        let dashboard = row_to_dashboard(&row)?;

        let panel_rows = sqlx::query(
            "SELECT id, dashboard_id, title, panel_type, query, database_name,
                    config, grid_x, grid_y, grid_w, grid_h, display_order
             FROM dashboard_panels
             WHERE tenant_id = $1 AND dashboard_id = $2
             ORDER BY display_order ASC",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let panels = panel_rows
            .iter()
            .map(row_to_panel)
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(Some(DashboardWithPanels { dashboard, panels }))
    }

    async fn update_dashboard_in_tenant(
        &self,
        tenant: TenantId,
        id: Uuid,
        patch: DashboardUpdate,
    ) -> std::result::Result<Dashboard, CatalogError> {
        // Fetch current row, scoped to tenant.
        let maybe_row = sqlx::query(
            "SELECT id, name, description, time_range_preset,
                    refresh_interval_seconds, created_at, updated_at
             FROM dashboards
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let Some(row) = maybe_row else {
            return Err(CatalogError::DashboardNotFound { id });
        };
        let current = row_to_dashboard(&row)?;

        // Apply scalar patches in memory.
        let new_name = patch.name.unwrap_or(current.name);
        let new_description = patch.description.unwrap_or(current.description);
        let new_time_range = patch.time_range_preset.unwrap_or(current.time_range_preset);
        let new_refresh = patch
            .refresh_interval_seconds
            .unwrap_or(current.refresh_interval_seconds);

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        // Update the dashboard row.
        let updated_row = sqlx::query(
            "UPDATE dashboards
             SET name = $3,
                 description = $4,
                 time_range_preset = $5,
                 refresh_interval_seconds = $6,
                 updated_at = now()
             WHERE tenant_id = $1 AND id = $2
             RETURNING id, name, description, time_range_preset,
                       refresh_interval_seconds, created_at, updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(&new_name)
        .bind(new_description.as_deref())
        .bind(&new_time_range)
        .bind(new_refresh)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        // Atomically replace panels if provided.
        if let Some(new_panels) = patch.panels {
            sqlx::query(
                "DELETE FROM dashboard_panels
                 WHERE tenant_id = $1 AND dashboard_id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

            insert_panels(&mut *tx, tenant, id, &new_panels).await?;
        }

        tx.commit()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        row_to_dashboard(&updated_row)
    }

    async fn delete_dashboard_in_tenant(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> std::result::Result<bool, CatalogError> {
        let result = sqlx::query("DELETE FROM dashboards WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    // --- graphs ---

    async fn create_graph_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
        spec: GraphSpec,
    ) -> std::result::Result<GraphRegistration, CatalogError> {
        let row = sqlx::query(
            "INSERT INTO graph_registrations
               (tenant_id, database, name, node_table, edge_table,
                id_col, label_col, src_col, dst_col, type_col, realm_col)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             RETURNING id, database, name, node_table, edge_table,
                       id_col, label_col, src_col, dst_col, type_col, realm_col,
                       created_at, updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .bind(&spec.node_table)
        .bind(&spec.edge_table)
        .bind(&spec.id_col)
        .bind(&spec.label_col)
        .bind(&spec.src_col)
        .bind(&spec.dst_col)
        .bind(&spec.type_col)
        .bind(spec.realm_col.as_deref())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        row_to_graph(&row)
    }

    async fn list_graphs_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
    ) -> std::result::Result<Vec<GraphRegistration>, CatalogError> {
        let rows = sqlx::query(
            "SELECT id, database, name, node_table, edge_table,
                    id_col, label_col, src_col, dst_col, type_col, realm_col,
                    created_at, updated_at
             FROM graph_registrations
             WHERE tenant_id = $1 AND database = $2
             ORDER BY name",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        rows.iter().map(row_to_graph).collect()
    }

    async fn get_graph_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
    ) -> std::result::Result<Option<GraphRegistration>, CatalogError> {
        let maybe = sqlx::query(
            "SELECT id, database, name, node_table, edge_table,
                    id_col, label_col, src_col, dst_col, type_col, realm_col,
                    created_at, updated_at
             FROM graph_registrations
             WHERE tenant_id = $1 AND database = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        match maybe {
            Some(row) => Ok(Some(row_to_graph(&row)?)),
            None => Ok(None),
        }
    }

    async fn drop_graph_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
    ) -> std::result::Result<bool, CatalogError> {
        let res = sqlx::query(
            "DELETE FROM graph_registrations WHERE tenant_id = $1 AND database = $2 AND name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    // --- auth: users ---

    async fn create_user_in_tenant(
        &self,
        tenant: TenantId,
        username: &str,
        password_hash: &str,
        role: &str,
    ) -> std::result::Result<User, CatalogError> {
        let row = sqlx::query(
            "INSERT INTO users (tenant_id, username, password_hash, role)
             VALUES ($1, $2, $3, $4)
             RETURNING id, username, role, created_at, updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(username)
        .bind(password_hash)
        .bind(role)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        row_to_user(&row)
    }

    async fn get_user_with_hash_in_tenant(
        &self,
        tenant: TenantId,
        username: &str,
    ) -> std::result::Result<Option<(User, String)>, CatalogError> {
        let maybe = sqlx::query(
            "SELECT id, username, role, password_hash, created_at, updated_at
             FROM users
             WHERE tenant_id = $1 AND username = $2",
        )
        .bind(tenant.as_uuid())
        .bind(username)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        let Some(row) = maybe else { return Ok(None) };
        use sqlx::Row as _;
        let hash: String = row.try_get("password_hash").map_err(sql_err)?;
        let user = row_to_user(&row)?;
        Ok(Some((user, hash)))
    }

    async fn count_users_in_tenant(
        &self,
        tenant: TenantId,
    ) -> std::result::Result<u64, CatalogError> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM users WHERE tenant_id = $1")
                .bind(tenant.as_uuid())
                .fetch_one(&self.pool)
                .await
                .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(count as u64)
    }

    // --- auth: api tokens ---

    async fn insert_api_token_in_tenant(
        &self,
        tenant: TenantId,
        token_hash: &[u8],
        scopes: &str,
        subject: Option<&str>,
        kind: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> std::result::Result<(), CatalogError> {
        sqlx::query(
            "INSERT INTO api_tokens (tenant_id, token_hash, scopes, subject, kind, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(tenant.as_uuid())
        .bind(token_hash)
        .bind(scopes)
        .bind(subject)
        .bind(kind)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(())
    }

    async fn lookup_api_token(
        &self,
        token_hash: &[u8],
    ) -> std::result::Result<Option<TokenPrincipal>, CatalogError> {
        let maybe = sqlx::query(
            "SELECT tenant_id, scopes, subject
             FROM api_tokens
             WHERE token_hash = $1
               AND revoked_at IS NULL
               AND kind <> 'refresh'
               AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let Some(row) = maybe else { return Ok(None) };

        use sqlx::Row as _;
        let tenant_uuid: Uuid = row.try_get("tenant_id").map_err(sql_err)?;
        let scopes: String = row.try_get("scopes").map_err(sql_err)?;
        let subject: Option<String> = row.try_get("subject").map_err(sql_err)?;

        // Resolve the highest-privilege role from the scopes string (mirrors db_backend.rs).
        let role = scopes
            .split(',')
            .map(str::trim)
            .filter(|s| matches!(*s, "admin" | "write" | "read"))
            .max_by_key(|s| match *s {
                "admin" => 2u8,
                "write" => 1,
                _ => 0,
            })
            .unwrap_or("read")
            .to_owned();

        // Fire-and-forget last_used_at update — ignore errors.
        let _ = sqlx::query(
            "UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1",
        )
        .bind(token_hash)
        .execute(&self.pool)
        .await;

        Ok(Some(TokenPrincipal {
            tenant: kyma_core::tenant::TenantId::from_uuid(tenant_uuid),
            role,
            subject,
        }))
    }

    async fn revoke_api_token(
        &self,
        token_hash: &[u8],
    ) -> std::result::Result<bool, CatalogError> {
        let res = sqlx::query(
            "UPDATE api_tokens
             SET revoked_at = now()
             WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(token_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(res.rows_affected() > 0)
    }

    async fn insert_session_token(
        &self,
        token_hash: &[u8],
        scopes: &str,
        subject: Option<&str>,
        kind: &str,
        expires_at: DateTime<Utc>,
        session_id: Uuid,
    ) -> std::result::Result<(), CatalogError> {
        sqlx::query(
            "INSERT INTO api_tokens (tenant_id, token_hash, scopes, subject, kind, expires_at, session_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(kyma_core::tenant::DEFAULT_TENANT.as_uuid())
        .bind(token_hash)
        .bind(scopes)
        .bind(subject)
        .bind(kind)
        .bind(expires_at)
        .bind(session_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(())
    }

    async fn lookup_refresh_token(
        &self,
        token_hash: &[u8],
    ) -> std::result::Result<Option<RefreshClaim>, CatalogError> {
        let maybe = sqlx::query(
            "SELECT tenant_id, scopes, subject, session_id
             FROM api_tokens
             WHERE token_hash = $1
               AND kind = 'refresh'
               AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let Some(row) = maybe else { return Ok(None) };

        use sqlx::Row as _;
        let tenant_uuid: Uuid = row.try_get("tenant_id").map_err(sql_err)?;
        let scopes: String = row.try_get("scopes").map_err(sql_err)?;
        let subject: Option<String> = row.try_get("subject").map_err(sql_err)?;
        let session_id: Option<Uuid> = row.try_get("session_id").map_err(sql_err)?;

        let role = scopes
            .split(',')
            .map(str::trim)
            .filter(|s| matches!(*s, "admin" | "write" | "read"))
            .max_by_key(|s| match *s {
                "admin" => 2u8,
                "write" => 1,
                _ => 0,
            })
            .unwrap_or("read")
            .to_owned();

        Ok(Some(RefreshClaim {
            tenant: kyma_core::tenant::TenantId::from_uuid(tenant_uuid),
            role,
            subject,
            session_id: session_id.unwrap_or_else(Uuid::nil),
        }))
    }

    async fn revoke_session_by_token(
        &self,
        token_hash: &[u8],
    ) -> std::result::Result<u64, CatalogError> {
        // Revoke every active token sharing this token's session_id, plus the
        // token itself (covers a session-less token where the subquery is NULL).
        let res = sqlx::query(
            "UPDATE api_tokens
             SET revoked_at = now()
             WHERE revoked_at IS NULL
               AND (token_hash = $1
                    OR session_id = (SELECT session_id FROM api_tokens WHERE token_hash = $1))",
        )
        .bind(token_hash)
        .execute(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(res.rows_affected())
    }

    // --- schema-listing ---

    async fn list_databases_in_tenant(
        &self,
        tenant: TenantId,
    ) -> std::result::Result<Vec<String>, kyma_core::errors::CatalogError> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM databases WHERE tenant_id = $1 ORDER BY name ASC",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn list_tables_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
    ) -> std::result::Result<Vec<String>, kyma_core::errors::CatalogError> {
        let exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM databases WHERE tenant_id = $1 AND name = $2)",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        if !exists.0 {
            return Err(CatalogError::DatabaseNotFound(database.to_string()));
        }
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT t.name FROM tables t
             JOIN databases d ON d.id = t.database_id AND d.tenant_id = $1
             WHERE t.tenant_id = $1 AND d.name = $2
             ORDER BY t.name ASC",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(rows.into_iter().map(|r| r.0).collect())
    }

    async fn get_table_columns_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        table: &str,
    ) -> std::result::Result<Vec<ColumnInfo>, kyma_core::errors::CatalogError> {
        let db_exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM databases WHERE tenant_id = $1 AND name = $2)",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        if !db_exists.0 {
            return Err(CatalogError::DatabaseNotFound(database.to_string()));
        }
        let row: Option<(serde_json::Value,)> = sqlx::query_as(
            "SELECT ss.arrow_schema
             FROM tables t
             JOIN databases d ON d.id = t.database_id AND d.tenant_id = $1
             JOIN schema_snapshots ss ON ss.id = t.schema_snapshot_id
             WHERE t.tenant_id = $1 AND d.name = $2 AND t.name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(table)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;

        let (schema_json,) = row.ok_or_else(|| CatalogError::TableNotFound {
            database: database.to_owned(),
            name: table.to_owned(),
        })?;

        let fields = schema_json
            .get("fields")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                CatalogError::Sql("malformed arrow_schema: missing fields array".into())
            })?;

        let mut columns = Vec::with_capacity(fields.len());
        for f in fields {
            let name = f
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CatalogError::Sql("field missing name".into()))?
                .to_owned();
            let col_type = f
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CatalogError::Sql("field missing type".into()))?
                .to_owned();
            let nullable = f.get("nullable").and_then(|v| v.as_bool()).unwrap_or(true);
            columns.push(ColumnInfo {
                name,
                r#type: col_type,
                nullable,
            });
        }
        Ok(columns)
    }
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

fn sql_err(e: sqlx::Error) -> CatalogError {
    CatalogError::Sql(e.to_string())
}

fn row_to_dashboard(row: &sqlx::postgres::PgRow) -> std::result::Result<Dashboard, CatalogError> {
    use sqlx::Row as _;
    Ok(Dashboard {
        id: row.try_get("id").map_err(sql_err)?,
        name: row.try_get("name").map_err(sql_err)?,
        description: row.try_get("description").map_err(sql_err)?,
        time_range_preset: row.try_get("time_range_preset").map_err(sql_err)?,
        refresh_interval_seconds: row.try_get("refresh_interval_seconds").map_err(sql_err)?,
        created_at: row.try_get("created_at").map_err(sql_err)?,
        updated_at: row.try_get("updated_at").map_err(sql_err)?,
    })
}

fn row_to_panel(row: &sqlx::postgres::PgRow) -> std::result::Result<DashboardPanel, CatalogError> {
    use sqlx::Row as _;
    Ok(DashboardPanel {
        id: row.try_get("id").map_err(sql_err)?,
        dashboard_id: row.try_get("dashboard_id").map_err(sql_err)?,
        title: row.try_get("title").map_err(sql_err)?,
        panel_type: row.try_get("panel_type").map_err(sql_err)?,
        query: row.try_get("query").map_err(sql_err)?,
        database_name: row.try_get("database_name").map_err(sql_err)?,
        config: row.try_get("config").map_err(sql_err)?,
        grid_x: row.try_get("grid_x").map_err(sql_err)?,
        grid_y: row.try_get("grid_y").map_err(sql_err)?,
        grid_w: row.try_get("grid_w").map_err(sql_err)?,
        grid_h: row.try_get("grid_h").map_err(sql_err)?,
        display_order: row.try_get("display_order").map_err(sql_err)?,
    })
}

fn row_to_user(row: &sqlx::postgres::PgRow) -> std::result::Result<User, CatalogError> {
    use sqlx::Row as _;
    Ok(User {
        id: row.try_get("id").map_err(sql_err)?,
        username: row.try_get("username").map_err(sql_err)?,
        role: row.try_get("role").map_err(sql_err)?,
        created_at: row.try_get("created_at").map_err(sql_err)?,
        updated_at: row.try_get("updated_at").map_err(sql_err)?,
    })
}

fn row_to_graph(row: &sqlx::postgres::PgRow) -> std::result::Result<GraphRegistration, CatalogError> {
    use sqlx::Row as _;
    Ok(GraphRegistration {
        id: row.try_get("id").map_err(sql_err)?,
        database: row.try_get("database").map_err(sql_err)?,
        name: row.try_get("name").map_err(sql_err)?,
        node_table: row.try_get("node_table").map_err(sql_err)?,
        edge_table: row.try_get("edge_table").map_err(sql_err)?,
        id_col: row.try_get("id_col").map_err(sql_err)?,
        label_col: row.try_get("label_col").map_err(sql_err)?,
        src_col: row.try_get("src_col").map_err(sql_err)?,
        dst_col: row.try_get("dst_col").map_err(sql_err)?,
        type_col: row.try_get("type_col").map_err(sql_err)?,
        realm_col: row.try_get("realm_col").map_err(sql_err)?,
        created_at: row.try_get("created_at").map_err(sql_err)?,
        updated_at: row.try_get("updated_at").map_err(sql_err)?,
    })
}

async fn insert_panels(
    tx: &mut sqlx::PgConnection,
    tenant: TenantId,
    dashboard_id: Uuid,
    panels: &[DashboardPanelInput],
) -> std::result::Result<(), CatalogError> {
    for panel in panels {
        let panel_id = panel.id.unwrap_or_else(Uuid::new_v4);
        sqlx::query(
            "INSERT INTO dashboard_panels
             (tenant_id, id, dashboard_id, title, panel_type, query, database_name,
              config, grid_x, grid_y, grid_w, grid_h, display_order)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(tenant.as_uuid())
        .bind(panel_id)
        .bind(dashboard_id)
        .bind(&panel.title)
        .bind(&panel.panel_type)
        .bind(panel.query.as_deref())
        .bind(panel.database_name.as_deref())
        .bind(&panel.config)
        .bind(panel.grid_x)
        .bind(panel.grid_y)
        .bind(panel.grid_w)
        .bind(panel.grid_h)
        .bind(panel.display_order)
        .execute(&mut *tx)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
    }
    Ok(())
}

fn role_to_str(role: NodeRole) -> &'static str {
    match role {
        NodeRole::AllInOne => "all_in_one",
        NodeRole::Ingest => "ingest",
        NodeRole::Query => "query",
        NodeRole::Compaction => "compaction",
    }
}

fn str_to_role(s: &str) -> NodeRole {
    match s {
        "ingest" => NodeRole::Ingest,
        "query" => NodeRole::Query,
        "compaction" => NodeRole::Compaction,
        _ => NodeRole::AllInOne,
    }
}

#[inline]
fn column_predicates_todo<T>(_: &T) {}

/// Serialise an Arrow `Schema` into a JSON representation durable in Postgres.
///
/// We use a self-describing shape rather than arrow's own schema-to-ipc
/// bytes so the catalog row is human-readable and tool-queryable.
fn schema_to_json(schema: &arrow_schema::Schema) -> Result<Json> {
    let fields: Vec<Json> = schema
        .fields()
        .iter()
        .map(|f| {
            serde_json::json!({
                "name":     f.name(),
                "type":     arrow_type_to_string(f.data_type()),
                "nullable": f.is_nullable(),
            })
        })
        .collect();
    Ok(serde_json::json!({ "fields": fields }))
}

fn json_to_schema(json: &Json) -> Result<Arc<arrow_schema::Schema>> {
    let fields = json
        .get("fields")
        .and_then(Json::as_array)
        .ok_or_else(|| CatalogError::Sql("malformed schema json: missing fields".into()))?;
    let mut out = Vec::with_capacity(fields.len());
    for f in fields {
        let name = f
            .get("name")
            .and_then(Json::as_str)
            .ok_or_else(|| CatalogError::Sql("field missing name".into()))?;
        let ty = f
            .get("type")
            .and_then(Json::as_str)
            .ok_or_else(|| CatalogError::Sql("field missing type".into()))?;
        let nullable = f.get("nullable").and_then(Json::as_bool).unwrap_or(true);
        out.push(arrow_schema::Field::new(
            name,
            string_to_arrow_type(ty)?,
            nullable,
        ));
    }
    Ok(Arc::new(arrow_schema::Schema::new(out)))
}

fn arrow_type_to_string(ty: &arrow_schema::DataType) -> String {
    use arrow_schema::{DataType, TimeUnit};
    match ty {
        DataType::Boolean => "bool".into(),
        DataType::Int32 => "int".into(),
        DataType::Int64 => "long".into(),
        DataType::Float64 => "real".into(),
        DataType::Utf8 | DataType::LargeUtf8 => "string".into(),
        DataType::Timestamp(TimeUnit::Nanosecond, _) => "timestamp".into(),
        DataType::Binary | DataType::LargeBinary => "dynamic".into(),
        DataType::FixedSizeList(inner, dim) if matches!(inner.data_type(), DataType::Float32) => {
            format!("vector({dim})")
        }
        other => format!("arrow:{other:?}"),
    }
}

fn string_to_arrow_type(s: &str) -> Result<arrow_schema::DataType> {
    use arrow_schema::{DataType, Field, TimeUnit};
    use std::sync::Arc;
    Ok(match s {
        "bool" => DataType::Boolean,
        "int" => DataType::Int32,
        "long" => DataType::Int64,
        "real" => DataType::Float64,
        "string" => DataType::Utf8,
        "timestamp" => DataType::Timestamp(TimeUnit::Nanosecond, None),
        "dynamic" => DataType::Binary,
        other if other.starts_with("vector(") && other.ends_with(')') => {
            let inner = &other[7..other.len() - 1];
            let dim: i32 = inner.trim().parse().map_err(|_| {
                CatalogError::Sql(format!(
                    "vector(N): N must be a positive integer, got '{inner}'"
                ))
            })?;
            if dim <= 0 {
                return Err(
                    CatalogError::Sql(format!("vector(N): N must be > 0, got {dim}")).into(),
                );
            }
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, false)), dim)
        }
        other => {
            return Err(
                CatalogError::Sql(format!("unsupported column type in schema: {other}")).into(),
            )
        }
    })
}
