//! Persistence for in-progress OAuth flows (`oauth_flows` table).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use kyma_core::tenant::TenantId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Fields for a new pending flow row.
pub struct NewFlow<'a> {
    pub tenant: TenantId,
    pub state: &'a str,
    pub provider: &'a str,
    pub data_source_type: &'a str,
    pub label: &'a str,
    pub scopes: &'a str,
    pub redirect_uri: &'a str,
    pub enc_code_verifier: Option<Vec<u8>>,
    pub expires_at: DateTime<Utc>,
}

/// Insert a new pending flow.
pub async fn insert_flow(pool: &PgPool, f: &NewFlow<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO oauth_flows
         (tenant_id, state, provider, data_source_type, label, scopes, redirect_uri,
          enc_code_verifier, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.state)
    .bind(f.provider)
    .bind(f.data_source_type)
    .bind(f.label)
    .bind(f.scopes)
    .bind(f.redirect_uri)
    .bind(&f.enc_code_verifier)
    .bind(f.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// A flow claimed for completion by the callback.
pub struct ClaimedFlow {
    pub tenant: TenantId,
    pub provider: String,
    pub data_source_type: String,
    pub label: String,
    pub scopes: String,
    pub redirect_uri: String,
    pub enc_code_verifier: Option<Vec<u8>>,
}

/// Atomically consume a pending, unexpired flow by its `state` token. Marks it
/// `consumed` (single-use) and returns its stored fields. Errors if the state
/// is unknown, expired, or already used — the CSRF defense for the
/// unauthenticated callback.
pub async fn consume_flow(pool: &PgPool, state: &str) -> Result<ClaimedFlow> {
    let row = sqlx::query(
        "UPDATE oauth_flows SET status = 'consumed'
         WHERE state = $1 AND status = 'pending' AND expires_at > now()
         RETURNING tenant_id, provider, data_source_type, label, scopes, redirect_uri,
                   enc_code_verifier",
    )
    .bind(state)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("invalid, expired, or already-used authorization state"))?;
    let tenant_uuid: Uuid = row.get("tenant_id");
    Ok(ClaimedFlow {
        tenant: TenantId::from_uuid(tenant_uuid),
        provider: row.get("provider"),
        data_source_type: row.get("data_source_type"),
        label: row.get("label"),
        scopes: row.get("scopes"),
        redirect_uri: row.get("redirect_uri"),
        enc_code_verifier: row.get("enc_code_verifier"),
    })
}

/// Mark a flow completed and record the credential it minted.
pub async fn mark_completed(pool: &PgPool, state: &str, credential_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE oauth_flows SET status = 'completed', credential_id = $2 WHERE state = $1")
        .bind(state)
        .bind(credential_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Mark a flow errored with a message (best-effort).
pub async fn mark_error(pool: &PgPool, state: &str, error: &str) {
    let _ = sqlx::query("UPDATE oauth_flows SET status = 'error', error = $2 WHERE state = $1")
        .bind(state)
        .bind(error)
        .execute(pool)
        .await;
}

/// Status + minted credential for a flow, scoped to the requesting tenant.
/// Used by the poll endpoint for the no-`postMessage` (Tauri / popup-blocked)
/// fallback.
pub async fn flow_status(
    pool: &PgPool,
    tenant: TenantId,
    state: &str,
) -> Result<Option<(String, Option<Uuid>)>> {
    let row = sqlx::query(
        "SELECT status, credential_id FROM oauth_flows WHERE state = $1 AND tenant_id = $2",
    )
    .bind(state)
    .bind(tenant.as_uuid())
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        let status: String = r.get("status");
        let cid: Option<Uuid> = r.get("credential_id");
        (status, cid)
    }))
}
