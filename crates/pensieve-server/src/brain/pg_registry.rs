//! Hosted-mode brain registry over Postgres (`brain_repos`, migration 034).

use async_trait::async_trait;
use pensieve_brain::registry::{BrainConfig, BrainRecord, BrainRegistry, BrainRuntime};
use pensieve_brain::BrainError;
use pensieve_core::tenant::TenantId;
use sqlx::PgPool;

pub struct PgBrainRegistry {
    pool: PgPool,
    tenant: TenantId,
}

impl PgBrainRegistry {
    pub fn new(pool: PgPool, tenant: TenantId) -> Self {
        Self { pool, tenant }
    }
}

fn db_err(e: sqlx::Error) -> BrainError {
    BrainError::Other(format!("brain_repos: {e}"))
}

fn decode(config: serde_json::Value, runtime: serde_json::Value) -> Result<BrainRecord, BrainError> {
    Ok(BrainRecord {
        config: serde_json::from_value(config)?,
        runtime: serde_json::from_value(runtime).unwrap_or_default(),
    })
}

#[async_trait]
impl BrainRegistry for PgBrainRegistry {
    async fn list(&self) -> Result<Vec<BrainRecord>, BrainError> {
        let rows: Vec<(serde_json::Value, serde_json::Value)> = sqlx::query_as(
            "SELECT config, runtime FROM brain_repos WHERE tenant_id = $1 ORDER BY name",
        )
        .bind(self.tenant.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        rows.into_iter().map(|(c, r)| decode(c, r)).collect()
    }

    async fn get(&self, name: &str) -> Result<Option<BrainRecord>, BrainError> {
        let row: Option<(serde_json::Value, serde_json::Value)> = sqlx::query_as(
            "SELECT config, runtime FROM brain_repos WHERE tenant_id = $1 AND name = $2",
        )
        .bind(self.tenant.as_uuid())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        row.map(|(c, r)| decode(c, r)).transpose()
    }

    async fn upsert_config(&self, cfg: &BrainConfig) -> Result<(), BrainError> {
        sqlx::query(
            "INSERT INTO brain_repos (tenant_id, name, config, runtime, updated_at) \
             VALUES ($1, $2, $3, '{}'::jsonb, now()) \
             ON CONFLICT (tenant_id, name) \
             DO UPDATE SET config = EXCLUDED.config, updated_at = now()",
        )
        .bind(self.tenant.as_uuid())
        .bind(&cfg.name)
        .bind(serde_json::to_value(cfg)?)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn delete(&self, name: &str) -> Result<(), BrainError> {
        sqlx::query("DELETE FROM brain_repos WHERE tenant_id = $1 AND name = $2")
            .bind(self.tenant.as_uuid())
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn update_runtime(&self, name: &str, rt: &BrainRuntime) -> Result<(), BrainError> {
        let updated = sqlx::query(
            "UPDATE brain_repos SET runtime = $3, updated_at = now() \
             WHERE tenant_id = $1 AND name = $2",
        )
        .bind(self.tenant.as_uuid())
        .bind(name)
        .bind(serde_json::to_value(rt)?)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        if updated.rows_affected() == 0 {
            return Err(BrainError::Other(format!("brain `{name}` not found")));
        }
        Ok(())
    }
}
