//! Per-tenant retention settings + application (Postgres-only).
//!
//! Load/save the `retention_settings` row, and apply the resolved retention:
//!   * `restamp_artifact_expiry` — stamps `artifacts.expires_at` so the
//!     ArtifactRetentionWorker actually sweeps expired blobs.
//!   * `reconcile_table_retention` — pushes explicit per-table overrides into
//!     `tables.config.retention_days`, which the existing RetentionSweeper reads.

use pensieve_core::errors::{CatalogError, Result};
use pensieve_core::retention::RetentionSettings;
use pensieve_core::tenant::TenantId;

use crate::PostgresCatalog;

fn sql(e: sqlx::Error) -> CatalogError {
    CatalogError::Sql(e.to_string())
}

impl PostgresCatalog {
    /// Tenants that have configured retention settings — the set the worker
    /// (re)stamps each tick so newly-written artifacts become sweepable.
    pub async fn retention_tenants(&self) -> Result<Vec<TenantId>> {
        let rows: Vec<(uuid::Uuid,)> =
            sqlx::query_as("SELECT tenant_id FROM retention_settings")
                .fetch_all(&self.pool)
                .await
                .map_err(sql)?;
        Ok(rows.into_iter().map(|(u,)| TenantId::from_uuid(u)).collect())
    }

    /// Load a tenant's retention settings; defaults (retain forever) if unset.
    pub async fn load_retention_settings(&self, tenant: TenantId) -> Result<RetentionSettings> {
        let row: Option<(serde_json::Value,)> =
            sqlx::query_as("SELECT settings FROM retention_settings WHERE tenant_id = $1")
                .bind(tenant.as_uuid())
                .fetch_optional(&self.pool)
                .await
                .map_err(sql)?;
        Ok(row
            .and_then(|(v,)| serde_json::from_value(v).ok())
            .unwrap_or_default())
    }

    /// Upsert a tenant's retention settings.
    pub async fn save_retention_settings(
        &self,
        tenant: TenantId,
        s: &RetentionSettings,
    ) -> Result<()> {
        let v = serde_json::to_value(s).map_err(|e| CatalogError::Sql(e.to_string()))?;
        sqlx::query(
            "INSERT INTO retention_settings (tenant_id, settings, updated_at)
             VALUES ($1, $2, now())
             ON CONFLICT (tenant_id) DO UPDATE SET settings = EXCLUDED.settings, updated_at = now()",
        )
        .bind(tenant.as_uuid())
        .bind(v)
        .execute(&self.pool)
        .await
        .map_err(sql)?;
        Ok(())
    }

    /// Re-stamp `expires_at` on every live artifact for `tenant` from the
    /// resolved retention (class → source → global). Idempotent; safe to run on
    /// every worker tick and on settings change. Returns rows touched.
    pub async fn restamp_artifact_expiry(
        &self,
        tenant: TenantId,
        s: &RetentionSettings,
    ) -> Result<u64> {
        let combos: Vec<(String, String)> = sqlx::query_as(
            "SELECT DISTINCT source, artifact_class FROM artifacts
             WHERE tenant_id = $1 AND deleted_at IS NULL",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;

        let mut updated = 0u64;
        for (source, class) in &combos {
            let res = match s.artifact_days(source, class) {
                Some(days) => sqlx::query(
                    "UPDATE artifacts
                     SET expires_at = created_at + make_interval(days => $4)
                     WHERE tenant_id = $1 AND deleted_at IS NULL
                       AND source = $2 AND artifact_class = $3
                       AND expires_at IS DISTINCT FROM created_at + make_interval(days => $4)",
                )
                .bind(tenant.as_uuid())
                .bind(source)
                .bind(class)
                .bind(days as i32)
                .execute(&self.pool)
                .await
                .map_err(sql)?,
                None => sqlx::query(
                    "UPDATE artifacts SET expires_at = NULL
                     WHERE tenant_id = $1 AND deleted_at IS NULL
                       AND source = $2 AND artifact_class = $3 AND expires_at IS NOT NULL",
                )
                .bind(tenant.as_uuid())
                .bind(source)
                .bind(class)
                .execute(&self.pool)
                .await
                .map_err(sql)?,
            };
            updated += res.rows_affected();
        }
        Ok(updated)
    }

    /// Push explicit per-table retention overrides into `tables.config`. The
    /// existing RetentionSweeper reads `config->>'retention_days'`. Global /
    /// source defaults are deliberately NOT applied to columnar tables — those
    /// target only the expendable artifact blobs, so a global default can never
    /// silently start expiring primary table data.
    pub async fn reconcile_table_retention(
        &self,
        tenant: TenantId,
        s: &RetentionSettings,
    ) -> Result<()> {
        for (table_ref, days) in &s.per_table_days {
            let Some((db, table)) = table_ref.split_once('.') else {
                continue;
            };
            sqlx::query(
                "UPDATE tables t
                 SET config = jsonb_set(coalesce(t.config, '{}'::jsonb), '{retention_days}', to_jsonb($1::int))
                 FROM databases d
                 WHERE t.database_id = d.id AND d.tenant_id = $2 AND d.name = $3 AND t.name = $4",
            )
            .bind(*days as i32)
            .bind(tenant.as_uuid())
            .bind(db)
            .bind(table)
            .execute(&self.pool)
            .await
            .map_err(sql)?;
        }
        Ok(())
    }
}
