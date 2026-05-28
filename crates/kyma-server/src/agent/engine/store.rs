//! Persistence for the active EngineConfig — singleton row in `engine_config`.

use super::{EngineConfig, EngineKind};
use async_trait::async_trait;
use sqlx::PgPool;
use std::str::FromStr;
use uuid::Uuid;

#[async_trait]
pub trait EnginePreferenceStore: Send + Sync {
    async fn get(&self) -> anyhow::Result<EngineConfig>;
    async fn put(&self, cfg: &EngineConfig) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct PgEnginePreferenceStore {
    pool: PgPool,
}

impl PgEnginePreferenceStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EnginePreferenceStore for PgEnginePreferenceStore {
    async fn get(&self) -> anyhow::Result<EngineConfig> {
        let row: (String, String, Option<Uuid>, Option<String>, serde_json::Value) = sqlx::query_as(
            "SELECT kind, model, credential_id, host, extras FROM engine_config WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        let kind = parse_kind(&row.0)?;
        Ok(EngineConfig {
            kind,
            model: row.1,
            credential_id: row.2,
            host: row.3,
            extras: row.4,
        })
    }

    async fn put(&self, cfg: &EngineConfig) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO engine_config (id, kind, model, credential_id, host, extras, updated_at)
             VALUES (1, $1, $2, $3, $4, $5, NOW())
             ON CONFLICT (id) DO UPDATE SET
                kind = EXCLUDED.kind,
                model = EXCLUDED.model,
                credential_id = EXCLUDED.credential_id,
                host = EXCLUDED.host,
                extras = EXCLUDED.extras,
                updated_at = NOW()",
        )
        .bind(cfg.kind.as_str())
        .bind(&cfg.model)
        .bind(cfg.credential_id)
        .bind(&cfg.host)
        .bind(&cfg.extras)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn parse_kind(s: &str) -> anyhow::Result<EngineKind> {
    Ok(match s {
        "anthropic" => EngineKind::Anthropic,
        "openai" => EngineKind::Openai,
        "ollama" => EngineKind::Ollama,
        "claude_cli" => EngineKind::ClaudeCli,
        other => anyhow::bail!("unknown engine kind: {other}"),
    })
}

impl FromStr for EngineKind {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_kind(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;

    async fn pool() -> Option<PgPool> {
        let url = std::env::var("DATABASE_URL").ok()?;
        PgPoolOptions::new().connect(&url).await.ok()
    }

    #[tokio::test]
    async fn roundtrip_through_postgres() {
        let Some(pool) = pool().await else {
            eprintln!("skip: DATABASE_URL not set");
            return;
        };
        let store = PgEnginePreferenceStore::new(pool);
        let cfg = EngineConfig {
            kind: EngineKind::Anthropic,
            model: "claude-sonnet-4-6".into(),
            credential_id: None,
            host: None,
            extras: serde_json::json!({"max_tokens": 4096}),
        };
        store.put(&cfg).await.unwrap();
        let back = store.get().await.unwrap();
        assert_eq!(back.kind, EngineKind::Anthropic);
        assert_eq!(back.model, "claude-sonnet-4-6");
        assert_eq!(back.extras["max_tokens"], 4096);
        store
            .put(&EngineConfig {
                kind: EngineKind::Ollama,
                model: "gemma4:latest".into(),
                credential_id: None,
                host: Some("http://localhost:11434".into()),
                extras: serde_json::json!({}),
            })
            .await
            .unwrap();
    }
}
