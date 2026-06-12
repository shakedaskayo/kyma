//! SQLite-backed data source catalog + control for local mode.
//!
//! `SqliteDataSourceCatalog` implements the abstract `DataSourceCatalog` trait
//! so the admin API and scheduler work over the embedded SQLite database.
//!
//! `SqliteDataSourceControl` implements the `DataSourceControl` trait for the
//! fabric executor path — it mirrors `PgDataSourceControl` but reads/writes
//! the SQLite `data_sources` / `data_source_cursors` tables.

use anyhow::Result;
use async_trait::async_trait;
use kyma_datasources::catalog_sql::DataSourceRow;
use kyma_datasources::catalog_trait::DataSourceCatalog;
use kyma_datasources::runner::DataSourceControl;
use kyma_core::tenant::TenantId;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// SqliteDataSourceCatalog
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct SqliteDataSourceCatalog {
    pool: SqlitePool,
}

impl SqliteDataSourceCatalog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DataSourceCatalog for SqliteDataSourceCatalog {
    async fn create_data_source(
        &self,
        tenant: TenantId,
        name: &str,
        type_id: &str,
        target_database: &str,
        target_table: &str,
        config: serde_json::Value,
        schedule_ms: i64,
        drive_model: &str,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now().to_rfc3339();
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        let config_str = serde_json::to_string(&config)?;
        sqlx::query(
            "INSERT INTO data_sources
               (id, tenant_id, name, type, target_database, target_table,
                config_json, schedule_ms, drive_model, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id_str)
        .bind(&tenant_str)
        .bind(name)
        .bind(type_id)
        .bind(target_database)
        .bind(target_table)
        .bind(&config_str)
        .bind(schedule_ms)
        .bind(drive_model)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    async fn list_data_sources(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<(Uuid, String, String, bool)>> {
        let tenant_str = tenant.as_uuid().to_string();
        let rows = sqlx::query(
            "SELECT id, name, type, enabled FROM data_sources
             WHERE tenant_id = ? ORDER BY name",
        )
        .bind(&tenant_str)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| {
                let id: String = r.get("id");
                let name: String = r.get("name");
                let type_id: String = r.get("type");
                let enabled: bool = r.get("enabled");
                Ok((id.parse().unwrap_or(Uuid::nil()), name, type_id, enabled))
            })
            .collect()
    }

    async fn load_data_source(&self, tenant: TenantId, id: Uuid) -> Result<Option<DataSourceRow>> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        let r = sqlx::query(
            "SELECT id, tenant_id, name, type, target_database, target_table,
                    config_json, schedule_ms, drive_model, enabled,
                    last_run_at, last_success_at, last_error,
                    last_rows_ingested, disabled_reason
             FROM data_sources WHERE tenant_id = ? AND id = ?",
        )
        .bind(&tenant_str)
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = r else {
            return Ok(None);
        };
        Ok(Some(row_to_ds(&r)?))
    }

    async fn patch_data_source(
        &self,
        tenant: TenantId,
        id: Uuid,
        name: Option<&str>,
        schedule_ms: Option<i64>,
        enabled: Option<bool>,
        config: Option<serde_json::Value>,
    ) -> Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let config_str = config.as_ref().map(|v| serde_json::to_string(v)).transpose()?;
        sqlx::query(
            "UPDATE data_sources SET
                 name = COALESCE(?, name),
                 schedule_ms = COALESCE(?, schedule_ms),
                 enabled = COALESCE(?, enabled),
                 config_json = COALESCE(?, config_json),
                 disabled_reason = CASE WHEN ? = 1 THEN NULL ELSE disabled_reason END,
                 updated_at = ?
             WHERE tenant_id = ? AND id = ?",
        )
        .bind(name)
        .bind(schedule_ms)
        .bind(enabled.map(|e| if e { 1i64 } else { 0i64 }))
        .bind(config_str.as_deref())
        .bind(enabled.map(|e| if e { 1i64 } else { 0i64 }))
        .bind(&now)
        .bind(&tenant_str)
        .bind(&id_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_data_source(&self, tenant: TenantId, id: Uuid) -> Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        sqlx::query("DELETE FROM data_sources WHERE tenant_id = ? AND id = ?")
            .bind(&tenant_str)
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn set_enabled(
        &self,
        tenant: TenantId,
        id: Uuid,
        enabled: bool,
        reason: Option<&str>,
    ) -> Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        if enabled {
            sqlx::query(
                "UPDATE data_sources SET enabled = 1, disabled_reason = NULL,
                 updated_at = ? WHERE tenant_id = ? AND id = ?",
            )
            .bind(&now)
            .bind(&tenant_str)
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE data_sources SET enabled = 0, disabled_reason = ?,
                 updated_at = ? WHERE tenant_id = ? AND id = ?",
            )
            .bind(reason)
            .bind(&now)
            .bind(&tenant_str)
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn enqueue_trigger(&self, tenant: TenantId, id: Uuid, now_ms: i64) -> Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        let schedule_ms: Option<i64> = sqlx::query_scalar(
            "SELECT schedule_ms FROM data_sources WHERE tenant_id = ? AND id = ?",
        )
        .bind(&tenant_str)
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await?;
        let key = if let Some(sched) = schedule_ms.filter(|&s| s > 0) {
            (now_ms / sched) * sched
        } else {
            now_ms
        };
        enqueue_jobs_sync(&self.pool, tenant, id, key).await?;
        Ok(())
    }

    async fn list_due_periodic(&self) -> Result<Vec<DataSourceRow>> {
        // SQLite uses integer seconds for interval arithmetic.
        let rows = sqlx::query(
            "SELECT id, tenant_id, name, type, target_database, target_table,
                    config_json, schedule_ms, drive_model, enabled,
                    last_run_at, last_success_at, last_error,
                    last_rows_ingested, disabled_reason
             FROM data_sources
             WHERE enabled = 1 AND drive_model = 'periodic'
               AND (last_run_at IS NULL
                    OR datetime(last_run_at) < datetime('now', '-' || (schedule_ms / 1000) || ' seconds'))",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(row_to_ds).collect()
    }

    async fn enqueue_data_source_sync(
        &self,
        tenant: TenantId,
        id: Uuid,
        bucket_ms: i64,
    ) -> Result<u64> {
        enqueue_jobs_sync(&self.pool, tenant, id, bucket_ms).await
    }
}

/// Insert a `data_source_sync` job into the local fabric `jobs` table.
/// Idempotent: the partial unique index returns 0 on conflict.
async fn enqueue_jobs_sync(
    pool: &SqlitePool,
    tenant: TenantId,
    id: Uuid,
    bucket_ms: i64,
) -> Result<u64> {
    let tenant_str = tenant.as_uuid().to_string();
    let id_str = id.to_string();
    let payload = serde_json::json!({
        "tenant_id": tenant_str,
        "data_source_id": id_str,
        "scheduled_for": bucket_ms.to_string(),
    });
    let payload_str = serde_json::to_string(&payload)?;
    let caps_str = serde_json::to_string(&["data_source"])?;
    let now = chrono::Utc::now().to_rfc3339();
    let job_id = Uuid::new_v4().to_string();
    let res = sqlx::query(
        "INSERT INTO jobs (id, tenant_id, kind, payload, priority, req_capabilities, created_at, updated_at)
         VALUES (?, ?, 'data_source_sync', ?, 0, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(&job_id)
    .bind(&tenant_str)
    .bind(&payload_str)
    .bind(&caps_str)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

fn row_to_ds(r: &sqlx::sqlite::SqliteRow) -> Result<DataSourceRow> {
    let id_str: String = r.get("id");
    let tenant_str: String = r.get("tenant_id");
    let config_str: String = r.get("config_json");
    let config_jsonb: serde_json::Value = serde_json::from_str(&config_str).unwrap_or(serde_json::Value::Null);
    let enabled_int: i64 = r.get("enabled");
    let last_run_str: Option<String> = r.get("last_run_at");
    let last_success_str: Option<String> = r.get("last_success_at");
    Ok(DataSourceRow {
        id: id_str.parse().unwrap_or(Uuid::nil()),
        tenant_id: TenantId::from_uuid(tenant_str.parse().unwrap_or(Uuid::nil())),
        name: r.get("name"),
        type_id: r.get("type"),
        target_database: r.get("target_database"),
        target_table: r.get("target_table"),
        config_jsonb,
        schedule_ms: r.get("schedule_ms"),
        drive_model: r.get("drive_model"),
        enabled: enabled_int != 0,
        last_run_at: last_run_str.and_then(|s| s.parse().ok()),
        last_success_at: last_success_str.and_then(|s| s.parse().ok()),
        last_error: r.get("last_error"),
        last_rows_ingested: r.get("last_rows_ingested"),
        disabled_reason: r.get("disabled_reason"),
    })
}

// ---------------------------------------------------------------------------
// SqliteDataSourceControl
// ---------------------------------------------------------------------------

/// SQLite implementation of the per-tick control surface — row + cursor +
/// status updates used by the fabric executor (`DataSourceSyncExecutor`).
#[derive(Clone)]
pub struct SqliteDataSourceControl {
    pool: SqlitePool,
}

impl SqliteDataSourceControl {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DataSourceControl for SqliteDataSourceControl {
    async fn load_data_source(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> anyhow::Result<Option<DataSourceRow>> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        let r = sqlx::query(
            "SELECT id, tenant_id, name, type, target_database, target_table,
                    config_json, schedule_ms, drive_model, enabled,
                    last_run_at, last_success_at, last_error,
                    last_rows_ingested, disabled_reason
             FROM data_sources WHERE tenant_id = ? AND id = ?",
        )
        .bind(&tenant_str)
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await?;
        let Some(r) = r else {
            return Ok(None);
        };
        Ok(Some(row_to_ds(&r)?))
    }

    async fn load_cursor(
        &self,
        tenant: TenantId,
        data_source_id: Uuid,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        let id_str = data_source_id.to_string();
        let _ = tenant; // cursor table keyed by data_source_id only in SQLite
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT cursor_json FROM data_source_cursors WHERE data_source_id = ?",
        )
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|(s,)| {
            if s.is_empty() {
                None
            } else {
                serde_json::from_str(&s).ok()
            }
        }))
    }

    async fn upsert_cursor(
        &self,
        tenant: TenantId,
        data_source_id: Uuid,
        cursor: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let _ = tenant;
        let id_str = data_source_id.to_string();
        let cursor_str = serde_json::to_string(cursor)?;
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO data_source_cursors (data_source_id, cursor_json, updated_at)
             VALUES (?, ?, ?)
             ON CONFLICT (data_source_id) DO UPDATE SET cursor_json = excluded.cursor_json,
             updated_at = excluded.updated_at",
        )
        .bind(&id_str)
        .bind(&cursor_str)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_run_success(
        &self,
        tenant: TenantId,
        data_source_id: Uuid,
        rows_ingested: i64,
    ) -> anyhow::Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = data_source_id.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE data_sources SET last_run_at = ?, last_success_at = ?,
             last_error = NULL, last_rows_ingested = ?, updated_at = ?
             WHERE tenant_id = ? AND id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(rows_ingested)
        .bind(&now)
        .bind(&tenant_str)
        .bind(&id_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn mark_run_failure(
        &self,
        tenant: TenantId,
        data_source_id: Uuid,
        error: &str,
    ) -> anyhow::Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = data_source_id.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE data_sources SET last_run_at = ?, last_error = ?, updated_at = ?
             WHERE tenant_id = ? AND id = ?",
        )
        .bind(&now)
        .bind(error)
        .bind(&now)
        .bind(&tenant_str)
        .bind(&id_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn disable_data_source(
        &self,
        tenant: TenantId,
        data_source_id: Uuid,
        reason: &str,
    ) -> anyhow::Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = data_source_id.to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "UPDATE data_sources SET enabled = 0, disabled_reason = ?, updated_at = ?
             WHERE tenant_id = ? AND id = ?",
        )
        .bind(reason)
        .bind(&now)
        .bind(&tenant_str)
        .bind(&id_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
