//! SQLite-backed credential store for local mode (`pensieve serve`).
//!
//! Mirrors the Postgres `PgCredentialStore` API — same AES-256-GCM envelope,
//! same CRUD surface — but persists to the embedded `credentials` table in the
//! local SQLite catalog instead of Postgres.

use anyhow::{Context, Result};
use async_trait::async_trait;
use pensieve_core::credentials::{Credential, CredentialStore, CredentialSummary, CredentialValue};
use pensieve_core::crypto::Crypto;
use pensieve_core::tenant::TenantId;
use serde_json::Value as Json;
use sqlx::SqlitePool;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct SqliteCredentialStore {
    pool: SqlitePool,
    crypto: Arc<Crypto>,
}

impl SqliteCredentialStore {
    pub fn new(pool: SqlitePool, crypto: Arc<Crypto>) -> Self {
        Self { pool, crypto }
    }
}

#[async_trait]
impl CredentialStore for SqliteCredentialStore {
    async fn get(&self, tenant: TenantId, id: Uuid) -> Result<Credential> {
        self.fetch(tenant, id).await
    }

    async fn update_value(&self, tenant: TenantId, id: Uuid, value: &CredentialValue) -> Result<()> {
        let plaintext = serde_json::to_vec(value)?;
        let enc = self.crypto.encrypt(&plaintext)?;
        let kind = value.kind();
        let id_str = id.to_string();
        let tenant_str = tenant.as_uuid().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let rows = sqlx::query(
            "UPDATE credentials SET enc_value = ?, kind = ?, updated_at = ?
             WHERE tenant_id = ? AND id = ?",
        )
        .bind(&enc)
        .bind(kind)
        .bind(&now)
        .bind(&tenant_str)
        .bind(&id_str)
        .execute(&self.pool)
        .await?;
        if rows.rows_affected() == 0 {
            anyhow::bail!("credential not found: {id}");
        }
        Ok(())
    }

    async fn create(
        &self,
        tenant: TenantId,
        label: &str,
        value: &CredentialValue,
        metadata: Json,
    ) -> Result<CredentialSummary> {
        let kind = value.kind();
        let plaintext = serde_json::to_vec(value)?;
        let enc = self.crypto.encrypt(&plaintext)?;
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let now_str = now.to_rfc3339();
        let id_str = id.to_string();
        let tenant_str = tenant.as_uuid().to_string();
        let meta_str = serde_json::to_string(&metadata)?;
        sqlx::query(
            "INSERT INTO credentials (id, tenant_id, label, kind, enc_value, metadata, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id_str)
        .bind(&tenant_str)
        .bind(label)
        .bind(kind)
        .bind(&enc)
        .bind(&meta_str)
        .bind(&now_str)
        .bind(&now_str)
        .execute(&self.pool)
        .await
        .context("inserting credential")?;
        Ok(CredentialSummary {
            id,
            label: label.into(),
            kind: kind.into(),
            preview: value.preview(),
            metadata,
            created_at: now,
            updated_at: now,
        })
    }

    async fn list(&self, tenant: TenantId) -> Result<Vec<CredentialSummary>> {
        let tenant_str = tenant.as_uuid().to_string();
        let rows = sqlx::query(
            "SELECT id, label, kind, enc_value, metadata, created_at, updated_at
             FROM credentials WHERE tenant_id = ? ORDER BY label",
        )
        .bind(&tenant_str)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            use sqlx::Row;
            let id: String = r.get("id");
            let label: String = r.get("label");
            let kind: String = r.get("kind");
            let enc: Vec<u8> = r.get("enc_value");
            let meta_str: String = r.get("metadata");
            let created_at_str: String = r.get("created_at");
            let updated_at_str: String = r.get("updated_at");
            let metadata: Json = serde_json::from_str(&meta_str).unwrap_or(Json::Object(Default::default()));
            let created_at = created_at_str.parse::<chrono::DateTime<chrono::Utc>>().unwrap_or_default();
            let updated_at = updated_at_str.parse::<chrono::DateTime<chrono::Utc>>().unwrap_or_default();
            let preview = match self.crypto.decrypt(&enc) {
                Ok(pt) => serde_json::from_slice::<CredentialValue>(&pt)
                    .map(|v| v.preview())
                    .unwrap_or_else(|_| "····".into()),
                Err(_) => "····".into(),
            };
            out.push(CredentialSummary {
                id: id.parse().unwrap_or(Uuid::nil()),
                label,
                kind,
                preview,
                metadata,
                created_at,
                updated_at,
            });
        }
        Ok(out)
    }

    async fn fetch(&self, tenant: TenantId, id: Uuid) -> Result<Credential> {
        use sqlx::Row;
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        let r = sqlx::query(
            "SELECT id, label, kind, enc_value, metadata, created_at, updated_at
             FROM credentials WHERE tenant_id = ? AND id = ?",
        )
        .bind(&tenant_str)
        .bind(&id_str)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("credential not found: {id}"))?;
        let enc: Vec<u8> = r.get("enc_value");
        let plaintext = self.crypto.decrypt(&enc)?;
        let value: CredentialValue = serde_json::from_slice(&plaintext)?;
        let meta_str: String = r.get("metadata");
        let metadata = serde_json::from_str(&meta_str).unwrap_or(Json::Object(Default::default()));
        let created_at = r
            .get::<String, _>("created_at")
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap_or_default();
        let updated_at = r
            .get::<String, _>("updated_at")
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap_or_default();
        Ok(Credential {
            id: r.get::<String, _>("id").parse().unwrap_or(Uuid::nil()),
            label: r.get("label"),
            kind: r.get("kind"),
            value,
            metadata,
            created_at,
            updated_at,
        })
    }

    async fn delete(&self, tenant: TenantId, id: Uuid) -> Result<()> {
        let tenant_str = tenant.as_uuid().to_string();
        let id_str = id.to_string();
        sqlx::query("DELETE FROM credentials WHERE tenant_id = ? AND id = ?")
            .bind(&tenant_str)
            .bind(&id_str)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
