//! Resolution of a provider's client-app credentials (`client_id` /
//! `client_secret`).
//!
//! Precedence (highest first):
//! 1. a per-tenant bring-your-own row in `oauth_clients` (encrypted secret),
//! 2. operator env `KYMA_OAUTH_<ENV_KEY>_CLIENT_ID` / `_CLIENT_SECRET`.

use anyhow::Result;
use kyma_core::crypto::Crypto;
use kyma_core::tenant::TenantId;
use sqlx::{PgPool, Row};

use super::provider::OAuthProvider;

/// A resolved client-app credential pair.
#[derive(Clone)]
pub struct ClientCreds {
    pub client_id: String,
    pub client_secret: String,
}

/// Resolve client creds for a tenant+provider, BYO row winning over env.
pub async fn resolve_client(
    pool: &PgPool,
    crypto: &Crypto,
    tenant: TenantId,
    provider: &OAuthProvider,
) -> Result<Option<ClientCreds>> {
    if let Some(c) = fetch_byo(pool, crypto, tenant, provider.slug).await? {
        return Ok(Some(c));
    }
    Ok(env_client(provider))
}

/// Operator-configured client creds from env, if both vars are present.
pub fn env_client(provider: &OAuthProvider) -> Option<ClientCreds> {
    let id = std::env::var(format!("KYMA_OAUTH_{}_CLIENT_ID", provider.env_key)).ok()?;
    let secret = std::env::var(format!("KYMA_OAUTH_{}_CLIENT_SECRET", provider.env_key)).ok()?;
    if id.is_empty() || secret.is_empty() {
        return None;
    }
    Some(ClientCreds {
        client_id: id,
        client_secret: secret,
    })
}

async fn fetch_byo(
    pool: &PgPool,
    crypto: &Crypto,
    tenant: TenantId,
    provider_slug: &str,
) -> Result<Option<ClientCreds>> {
    let row = sqlx::query(
        "SELECT client_id, enc_secret FROM oauth_clients
         WHERE tenant_id = $1 AND provider = $2",
    )
    .bind(tenant.as_uuid())
    .bind(provider_slug)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let client_id: String = row.get("client_id");
    let enc: Vec<u8> = row.get("enc_secret");
    let secret = String::from_utf8(crypto.decrypt(&enc)?)?;
    Ok(Some(ClientCreds {
        client_id,
        client_secret: secret,
    }))
}

/// Upsert a per-tenant bring-your-own client app (secret stored encrypted).
pub async fn upsert_byo(
    pool: &PgPool,
    crypto: &Crypto,
    tenant: TenantId,
    provider_slug: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<()> {
    let enc = crypto.encrypt(client_secret.as_bytes())?;
    sqlx::query(
        "INSERT INTO oauth_clients (tenant_id, provider, client_id, enc_secret)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (tenant_id, provider)
         DO UPDATE SET client_id = EXCLUDED.client_id,
                       enc_secret = EXCLUDED.enc_secret,
                       updated_at = now()",
    )
    .bind(tenant.as_uuid())
    .bind(provider_slug)
    .bind(client_id)
    .bind(&enc)
    .execute(pool)
    .await?;
    Ok(())
}
