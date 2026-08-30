//! Postgres-backed enabled-skills storage (one global row in `enabled_skills`).

use async_trait::async_trait;
use sqlx::PgPool;

#[async_trait]
pub trait EnabledSkillsStore: Send + Sync {
    async fn get(&self) -> anyhow::Result<Vec<String>>;
    async fn put(&self, skills: &[String]) -> anyhow::Result<()>;
}

#[derive(Clone)]
pub struct PgEnabledSkillsStore {
    pool: PgPool,
}

impl PgEnabledSkillsStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EnabledSkillsStore for PgEnabledSkillsStore {
    async fn get(&self) -> anyhow::Result<Vec<String>> {
        let row: (Vec<String>,) = sqlx::query_as(
            "SELECT skills FROM enabled_skills WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    async fn put(&self, skills: &[String]) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO enabled_skills (id, skills, updated_at) VALUES (1, $1, NOW())
             ON CONFLICT (id) DO UPDATE SET skills = EXCLUDED.skills, updated_at = NOW()",
        )
        .bind(skills)
        .execute(&self.pool)
        .await?;
        Ok(())
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
        let store = PgEnabledSkillsStore::new(pool);
        store
            .put(&["alpha".into(), "beta".into()])
            .await
            .unwrap();
        let back = store.get().await.unwrap();
        assert!(back.contains(&"alpha".to_string()));
        assert!(back.contains(&"beta".to_string()));
        store.put(&[]).await.unwrap();
        let cleared = store.get().await.unwrap();
        assert!(cleared.is_empty());
    }
}
