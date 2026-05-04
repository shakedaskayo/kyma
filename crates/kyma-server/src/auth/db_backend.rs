//! Postgres-backed token auth for the cloud control plane.
//!
//! Reads `api_tokens` rows shared with the cloud control plane:
//! ```sql
//! CREATE TABLE api_tokens (
//!     id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
//!     tenant_id     uuid NOT NULL,
//!     token_hash    bytea NOT NULL UNIQUE,  -- SHA-256(presented_token)
//!     scopes        text NOT NULL,           -- comma-separated; "admin" | "write" | "read"
//!     subject       text,
//!     last_used_at  timestamptz,
//!     revoked_at    timestamptz,
//!     created_at    timestamptz NOT NULL DEFAULT now()
//! );
//! ```

use super::backend::{AuthBackend, AuthError, Principal, Role};
use async_trait::async_trait;
use kyma_core::tenant::TenantId;
use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct DbAuthBackend {
    pool: PgPool,
}

impl DbAuthBackend {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn hash_token(token: &str) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(token.as_bytes());
        h.finalize().to_vec()
    }
}

#[async_trait]
impl AuthBackend for DbAuthBackend {
    fn enabled(&self) -> bool {
        true
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        let hash = Self::hash_token(token);
        let row = sqlx::query(
            "SELECT tenant_id, scopes, subject FROM api_tokens
             WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Backend(e.to_string()))?
        .ok_or(AuthError::UnknownToken)?;

        let tenant_uuid: uuid::Uuid = row
            .try_get("tenant_id")
            .map_err(|e| AuthError::Backend(e.to_string()))?;
        let scopes: String = row
            .try_get("scopes")
            .map_err(|e| AuthError::Backend(e.to_string()))?;
        let subject: Option<String> = row
            .try_get("subject")
            .map_err(|e| AuthError::Backend(e.to_string()))?;

        let role = scopes
            .split(',')
            .filter_map(|s| Role::parse(s.trim()))
            .max()
            .ok_or_else(|| {
                AuthError::Backend(format!(
                    "api_tokens row has no parseable scope: {scopes}"
                ))
            })?;

        let _ = sqlx::query(
            "UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1",
        )
        .bind(&hash)
        .execute(&self.pool)
        .await;

        Ok(Principal {
            tenant: TenantId::from_uuid(tenant_uuid),
            role,
            subject,
        })
    }
}
