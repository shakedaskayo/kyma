//! `PgSnapshotTxn` — a catalog snapshot transaction.
//!
//! Accumulates add/remove-extent operations in memory, then commits them in
//! a single short Postgres transaction. The final step is an optimistic
//! compare-and-set on `tables.current_snapshot_id`:
//!
//! ```sql
//! UPDATE tables SET current_snapshot_id = $new_snap
//!   WHERE id = $table_id AND current_snapshot_id = $parent_snap;
//! ```
//!
//! If zero rows are affected, another writer committed a newer snapshot
//! between `begin_snapshot` and `commit` — we return
//! [`CatalogError::Conflict`][kyma_core::errors::CatalogError::Conflict]
//! and the caller re-lineages and retries.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use kyma_core::catalog::{ExtentManifest, SnapshotSummary, SnapshotTxn};
use kyma_core::errors::{CatalogError, Error, Result};
use kyma_core::types::{ExtentId, SchemaSnapshotId, SnapshotId, TableId};
use sqlx::error::ErrorKind;
use sqlx::PgPool;
use uuid::Uuid;

/// In-memory accumulation of snapshot operations prior to commit.
#[derive(Debug)]
pub struct PgSnapshotTxn {
    pool: PgPool,
    table_id: TableId,
    parent_snapshot_id: SnapshotId,
    schema_snapshot_id: SchemaSnapshotId,
    added: Vec<ExtentManifest>,
    removed: Vec<ExtentId>,
}

impl PgSnapshotTxn {
    pub fn new(
        pool: PgPool,
        table_id: TableId,
        parent_snapshot_id: SnapshotId,
        schema_snapshot_id: SchemaSnapshotId,
    ) -> Self {
        Self {
            pool,
            table_id,
            parent_snapshot_id,
            schema_snapshot_id,
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
}

#[async_trait]
impl SnapshotTxn for PgSnapshotTxn {
    fn parent_snapshot_id(&self) -> SnapshotId {
        self.parent_snapshot_id
    }

    async fn add_extent(&mut self, manifest: ExtentManifest) -> Result<()> {
        self.added.push(manifest);
        Ok(())
    }

    async fn remove_extents(&mut self, extents: &[ExtentId]) -> Result<()> {
        self.removed.extend_from_slice(extents);
        Ok(())
    }

    async fn commit(self: Box<Self>, summary: SnapshotSummary) -> Result<SnapshotId> {
        let Self {
            pool,
            table_id,
            parent_snapshot_id,
            schema_snapshot_id,
            added,
            removed,
        } = *self;

        let summary_json = serde_json::json!({
            "operation":      summary.operation,
            "rows_added":     summary.rows_added,
            "rows_removed":   summary.rows_removed,
            "bytes_added":    summary.bytes_added,
            "bytes_removed":  summary.bytes_removed,
        });

        let mut tx = pool
            .begin()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        // Compute the new sequence number from the parent's sequence + 1.
        // We read via the tx to avoid torn reads under high concurrency.
        let parent_seq: i64 =
            sqlx::query_scalar("SELECT sequence_number FROM snapshots WHERE id = $1 FOR UPDATE")
                .bind(parent_snapshot_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(sql_err)?
                .ok_or_else(|| {
                    CatalogError::Sql(format!("parent snapshot {parent_snapshot_id} not found"))
                })?;
        let new_seq = parent_seq + 1;

        // 1. Insert the new snapshot row.
        //
        // Concurrency note: under READ COMMITTED, two racing txns can both
        // compute the same `new_seq` (parent_seq + 1). Postgres's unique
        // constraint on (table_id, sequence_number) catches the loser here —
        // before the tables-CAS at step 5. We translate that unique violation
        // to `CatalogError::Conflict` so callers handle the race uniformly
        // regardless of which line detected it.
        let (new_snap_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO snapshots (table_id, parent_id, sequence_number, schema_snapshot_id, summary)
             VALUES ($1, $2, $3, $4, $5) RETURNING id",
        )
        .bind(table_id.as_uuid())
        .bind(parent_snapshot_id.as_uuid())
        .bind(new_seq)
        .bind(schema_snapshot_id.as_uuid())
        .bind(&summary_json)
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_err_or_conflict)?;

        // 2. Insert a manifest row for this snapshot (kind=data).
        let (manifest_id,): (Uuid,) = sqlx::query_as(
            "INSERT INTO manifests (snapshot_id, kind, extent_count, byte_size)
             VALUES ($1, 'data', $2, $3) RETURNING id",
        )
        .bind(new_snap_id)
        .bind(added.len() as i32)
        .bind(added.iter().map(|e| e.byte_size as i64).sum::<i64>())
        .fetch_one(&mut *tx)
        .await
        .map_err(sql_err)?;

        // 3. Insert the added extents.
        for e in &added {
            sqlx::query(
                "INSERT INTO extents (
                    id, table_id, manifest_id, schema_snapshot_id, object_path, byte_size,
                    row_count, min_timestamp, max_timestamp, column_stats, present_paths,
                    compaction_gen, created_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
            )
            .bind(e.id.as_uuid())
            .bind(e.table_id.as_uuid())
            .bind(manifest_id)
            .bind(e.schema_snapshot_id.as_uuid())
            .bind(&e.object_path)
            .bind(e.byte_size as i64)
            .bind(e.row_count as i64)
            .bind(e.min_timestamp)
            .bind(e.max_timestamp)
            .bind(&e.column_stats)
            .bind(&e.present_paths)
            .bind(e.compaction_gen as i32)
            .bind(e.created_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_err)?;
        }

        // 4. Soft-delete the removed extents (compaction + retention path).
        if !removed.is_empty() {
            let ids: Vec<Uuid> = removed.iter().map(|e| *e.as_uuid()).collect();
            let now: DateTime<Utc> = Utc::now();
            sqlx::query(
                "UPDATE extents SET deleted_at = $1 WHERE id = ANY($2) AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(&ids)
            .execute(&mut *tx)
            .await
            .map_err(sql_err)?;
        }

        // 5. The critical CAS. If someone else committed since we read the
        //    parent, this UPDATE affects zero rows and we bail with Conflict.
        let swapped = sqlx::query(
            "UPDATE tables
             SET current_snapshot_id = $1
             WHERE id = $2 AND current_snapshot_id = $3",
        )
        .bind(new_snap_id)
        .bind(table_id.as_uuid())
        .bind(parent_snapshot_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(sql_err)?;

        if swapped.rows_affected() != 1 {
            // Rollback is implicit when `tx` drops without commit.
            return Err(CatalogError::Conflict.into());
        }

        tx.commit()
            .await
            .map_err(|e| CatalogError::Sql(e.to_string()))?;

        Ok(SnapshotId::from_uuid(new_snap_id))
    }

    async fn rollback(self: Box<Self>) -> Result<()> {
        // No pg tx is open until commit; drop is a no-op.
        Ok(())
    }
}

fn sql_err(e: sqlx::Error) -> CatalogError {
    CatalogError::Sql(e.to_string())
}

/// Unique-constraint violations on the snapshots table are semantically
/// conflicts — another writer committed a child snapshot with the same
/// sequence number in the window between our `SELECT` and our `INSERT`.
/// Returns [`Error`] (not `CatalogError`) so the caller's `?` lands on the
/// top-level `Result<T, Error>` without further conversion.
fn sql_err_or_conflict(e: sqlx::Error) -> Error {
    if let sqlx::Error::Database(db_err) = &e {
        if db_err.kind() == ErrorKind::UniqueViolation {
            return Error::Catalog(CatalogError::Conflict);
        }
    }
    Error::Catalog(CatalogError::Sql(e.to_string()))
}
