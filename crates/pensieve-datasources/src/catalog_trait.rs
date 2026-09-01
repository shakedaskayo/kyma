//! Abstract catalog interface for data source CRUD + scheduling.
//!
//! `DataSourceCatalog` decouples the admin API and scheduler from the concrete
//! `PostgresCatalog`/`PgPool` types so that local mode (SQLite) can supply its
//! own implementation without forking the handler logic.

use crate::catalog_sql::{self, DataSourceRow};
use anyhow::Result;
use pensieve_core::tenant::TenantId;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait DataSourceCatalog: Send + Sync {
    // ---- CRUD ----------------------------------------------------------------

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
    ) -> Result<Uuid>;

    async fn list_data_sources(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<(Uuid, String, String, bool)>>;

    async fn load_data_source(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> Result<Option<DataSourceRow>>;

    /// Partial update — `None` fields are left unchanged.
    async fn patch_data_source(
        &self,
        tenant: TenantId,
        id: Uuid,
        name: Option<&str>,
        schedule_ms: Option<i64>,
        enabled: Option<bool>,
        config: Option<serde_json::Value>,
    ) -> Result<()>;

    async fn delete_data_source(&self, tenant: TenantId, id: Uuid) -> Result<()>;

    /// Set the `enabled` flag; `reason` is stored in `disabled_reason` (cleared
    /// when re-enabling).
    async fn set_enabled(
        &self,
        tenant: TenantId,
        id: Uuid,
        enabled: bool,
        reason: Option<&str>,
    ) -> Result<()>;

    /// Look up `schedule_ms` for `id`, compute the time-bucket from `now_ms`,
    /// then enqueue a tick (idempotent via ON CONFLICT DO NOTHING).
    async fn enqueue_trigger(&self, tenant: TenantId, id: Uuid, now_ms: i64) -> Result<()>;

    /// List file-watcher registrations (infrastructure-level, not per-tenant).
    /// Default: returns an empty list — SQLite / test implementations don't need watchers.
    async fn list_watchers(&self) -> Result<Vec<serde_json::Value>> {
        Ok(vec![])
    }

    // ---- Scheduler -----------------------------------------------------------

    /// All enabled, periodic data sources due for a tick (cluster-global scan).
    async fn list_due_periodic(&self) -> Result<Vec<DataSourceRow>>;

    /// Enqueue a `data_source_sync` fabric job (idempotent). Returns 1 if
    /// inserted, 0 if the bucket already had a pending job.
    async fn enqueue_data_source_sync(
        &self,
        tenant: TenantId,
        id: Uuid,
        bucket_ms: i64,
    ) -> Result<u64>;
}

// ---------------------------------------------------------------------------
// Postgres implementation
// ---------------------------------------------------------------------------

/// Postgres-backed implementation backed by a `PgPool`.
#[derive(Clone)]
pub struct PgDataSourceCatalog {
    pool: PgPool,
}

impl PgDataSourceCatalog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn from_pg_catalog(catalog: &Arc<pensieve_catalog::PostgresCatalog>) -> Self {
        Self {
            pool: catalog.pool().clone(),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait::async_trait]
impl DataSourceCatalog for PgDataSourceCatalog {
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
        catalog_sql::create_data_source_direct(
            &self.pool,
            tenant,
            name,
            type_id,
            target_database,
            target_table,
            config,
            schedule_ms,
            drive_model,
        )
        .await
        .map_err(Into::into)
    }

    async fn list_data_sources(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<(Uuid, String, String, bool)>> {
        let rows = sqlx::query_as::<_, (Uuid, String, String, bool)>(
            "SELECT id, name, type, enabled FROM data_sources
             WHERE tenant_id = $1
             ORDER BY name",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    async fn load_data_source(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> Result<Option<DataSourceRow>> {
        catalog_sql::load_data_source(&self.pool, tenant, id)
            .await
            .map_err(Into::into)
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
        sqlx::query(
            "UPDATE data_sources SET
                 name = COALESCE($3, name),
                 schedule_ms = COALESCE($4, schedule_ms),
                 enabled = COALESCE($5, enabled),
                 config_jsonb = COALESCE($6, config_jsonb),
                 disabled_reason = CASE WHEN $5 = TRUE THEN NULL ELSE disabled_reason END,
                 updated_at = now()
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(name)
        .bind(schedule_ms)
        .bind(enabled)
        .bind(config.as_ref())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_data_source(&self, tenant: TenantId, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM data_sources WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(id)
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
        if enabled {
            sqlx::query(
                "UPDATE data_sources SET enabled = TRUE, disabled_reason = NULL,
                 updated_at = now() WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE data_sources SET enabled = FALSE, disabled_reason = $3,
                 updated_at = now() WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant.as_uuid())
            .bind(id)
            .bind(reason)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn enqueue_trigger(&self, tenant: TenantId, id: Uuid, now_ms: i64) -> Result<()> {
        let schedule_ms: Option<i64> = sqlx::query_scalar(
            "SELECT schedule_ms FROM data_sources WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        let key = if let Some(sched) = schedule_ms.filter(|&s| s > 0) {
            (now_ms / sched) * sched
        } else {
            now_ms
        };
        let _ = catalog_sql::enqueue_tick(&self.pool, tenant, id, key).await?;
        Ok(())
    }

    async fn list_watchers(&self) -> Result<Vec<serde_json::Value>> {
        let rows = crate::watchers::WatcherRegistry::list_and_prune(&self.pool).await?;
        rows.into_iter()
            .map(|r| serde_json::to_value(r).map_err(Into::into))
            .collect()
    }

    async fn list_due_periodic(&self) -> Result<Vec<DataSourceRow>> {
        catalog_sql::list_due_periodic(&self.pool)
            .await
            .map_err(Into::into)
    }

    async fn enqueue_data_source_sync(
        &self,
        tenant: TenantId,
        id: Uuid,
        bucket_ms: i64,
    ) -> Result<u64> {
        catalog_sql::enqueue_data_source_sync(&self.pool, tenant, id, bucket_ms)
            .await
            .map_err(Into::into)
    }
}
