//! Postgres-backed credential store.
//!
//! The `credentials` table holds tenant-scoped, encrypted secret values
//! (`enc_value = nonce(12) || ciphertext(AES-256-GCM)`). All reads decrypt on
//! the fly with the [`Crypto`] key loaded from `PENSIEVE_SECRET_KEY` at server
//! start, so plaintext never leaves memory unless a caller asks for it.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pensieve_core::credentials::{Credential, CredentialStore, CredentialSummary, CredentialValue};
use pensieve_core::crypto::Crypto;
use pensieve_core::tenant::TenantId;
use serde_json::Value as Json;
use sqlx::{PgPool, Row};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct PgCredentialStore {
    pool: PgPool,
    crypto: Arc<Crypto>,
}

impl PgCredentialStore {
    pub fn new(pool: PgPool, crypto: Arc<Crypto>) -> Self {
        Self { pool, crypto }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Insert a new credential. The `value` is JSON-encoded then encrypted.
    pub async fn create(
        &self,
        tenant: TenantId,
        label: &str,
        value: &CredentialValue,
        metadata: Json,
    ) -> Result<CredentialSummary> {
        let kind = value.kind();
        let plaintext = serde_json::to_vec(value)?;
        let enc = self.crypto.encrypt(&plaintext)?;
        let row = sqlx::query(
            "INSERT INTO credentials (tenant_id, label, kind, enc_value, metadata)
             VALUES ($1, $2, $3, $4, $5)
             RETURNING id, created_at, updated_at",
        )
        .bind(tenant.as_uuid())
        .bind(label)
        .bind(kind)
        .bind(&enc)
        .bind(&metadata)
        .fetch_one(&self.pool)
        .await?;
        let id: Uuid = row.get("id");
        let created_at: DateTime<Utc> = row.get("created_at");
        let updated_at: DateTime<Utc> = row.get("updated_at");
        Ok(CredentialSummary {
            id,
            label: label.into(),
            kind: kind.into(),
            preview: value.preview(),
            metadata,
            created_at,
            updated_at,
        })
    }

    pub async fn list(&self, tenant: TenantId) -> Result<Vec<CredentialSummary>> {
        let rows = sqlx::query(
            "SELECT id, label, kind, enc_value, metadata, created_at, updated_at
             FROM credentials
             WHERE tenant_id = $1
             ORDER BY label",
        )
        .bind(tenant.as_uuid())
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: Uuid = r.get("id");
            let label: String = r.get("label");
            let kind: String = r.get("kind");
            let enc: Vec<u8> = r.get("enc_value");
            let metadata: Json = r.get("metadata");
            let created_at: DateTime<Utc> = r.get("created_at");
            let updated_at: DateTime<Utc> = r.get("updated_at");
            // Decrypt only to compute the masked preview — never returned in plaintext.
            let preview = match self.crypto.decrypt(&enc) {
                Ok(pt) => serde_json::from_slice::<CredentialValue>(&pt)
                    .map(|v| v.preview())
                    .unwrap_or_else(|_| "····".into()),
                Err(_) => "····".into(),
            };
            out.push(CredentialSummary {
                id,
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

    /// Load and decrypt a credential by id. The caller must be inside the
    /// tenant boundary already (no impl-side cross-tenant resolution).
    pub async fn fetch(&self, tenant: TenantId, id: Uuid) -> Result<Credential> {
        let row = sqlx::query(
            "SELECT id, label, kind, enc_value, metadata, created_at, updated_at
             FROM credentials
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("credential not found: {id}"))?;
        let enc: Vec<u8> = row.get("enc_value");
        let plaintext = self.crypto.decrypt(&enc)?;
        let value: CredentialValue = serde_json::from_slice(&plaintext)?;
        Ok(Credential {
            id: row.get("id"),
            label: row.get("label"),
            kind: row.get("kind"),
            value,
            metadata: row.get("metadata"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
        })
    }

    pub async fn delete(&self, tenant: TenantId, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM credentials WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Re-encrypt and overwrite a credential's secret value in place (same id,
    /// new `enc_value`). Used by the OAuth2 token-refresh path to persist a
    /// rotated access/refresh token.
    pub async fn set_value(
        &self,
        tenant: TenantId,
        id: Uuid,
        value: &CredentialValue,
    ) -> Result<()> {
        let plaintext = serde_json::to_vec(value)?;
        let enc = self.crypto.encrypt(&plaintext)?;
        let res = sqlx::query(
            "UPDATE credentials
             SET enc_value = $3, kind = $4, updated_at = now()
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tenant.as_uuid())
        .bind(id)
        .bind(&enc)
        .bind(value.kind())
        .execute(&self.pool)
        .await?;
        if res.rows_affected() == 0 {
            anyhow::bail!("credential not found: {id}");
        }
        Ok(())
    }
}

#[async_trait]
impl CredentialStore for PgCredentialStore {
    async fn get(&self, tenant: TenantId, id: Uuid) -> Result<Credential> {
        self.fetch(tenant, id).await
    }

    async fn update_value(
        &self,
        tenant: TenantId,
        id: Uuid,
        value: &CredentialValue,
    ) -> Result<()> {
        self.set_value(tenant, id, value).await
    }

    async fn create(
        &self,
        tenant: TenantId,
        label: &str,
        value: &CredentialValue,
        metadata: serde_json::Value,
    ) -> Result<CredentialSummary> {
        PgCredentialStore::create(self, tenant, label, value, metadata).await
    }

    async fn list(&self, tenant: TenantId) -> Result<Vec<CredentialSummary>> {
        PgCredentialStore::list(self, tenant).await
    }

    async fn fetch(&self, tenant: TenantId, id: Uuid) -> Result<Credential> {
        PgCredentialStore::fetch(self, tenant, id).await
    }

    async fn delete(&self, tenant: TenantId, id: Uuid) -> Result<()> {
        PgCredentialStore::delete(self, tenant, id).await
    }
}
