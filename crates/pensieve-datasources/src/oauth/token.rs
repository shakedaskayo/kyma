//! Run-time access-token resolution for OAuth data sources.
//!
//! Every OAuth data source calls [`valid_access_token`] in `run_once` instead of
//! reading the credential directly — mirroring how the GitHub data source resolves
//! a PAT, but with transparent refresh + write-back for expired tokens.

use chrono::{Duration, Utc};
use pensieve_core::credentials::CredentialValue;
use uuid::Uuid;

use super::{client, flow, provider};
use crate::types::{DataSourceCtx, DataSourceError};

/// Resolve a valid OAuth2 access token for `credential_id`. If the stored token
/// is expired (or within 60s of it) and a refresh token is present, refresh via
/// the provider's token endpoint and persist the rotated tokens in place.
///
/// Providers that issue non-expiring tokens (Notion, Slack) — or any credential
/// without a refresh token — simply return the stored access token.
pub async fn valid_access_token(
    ctx: &DataSourceCtx,
    credential_id: Uuid,
) -> Result<String, DataSourceError> {
    let cred = ctx
        .credentials
        .get(ctx.tenant, credential_id)
        .await
        .map_err(|e| DataSourceError::Permanent(format!("resolve credential {credential_id}: {e}")))?;

    let CredentialValue::Oauth2 {
        access_token,
        refresh_token,
        expires_at,
    } = cred.value
    else {
        return Err(DataSourceError::Permanent(format!(
            "credential {credential_id} is kind={}; oauth data source requires `oauth2`",
            cred.kind
        )));
    };

    // Fresh enough? (No expiry recorded ⇒ treat as long-lived.)
    let fresh = expires_at.map_or(true, |e| e > Utc::now() + Duration::seconds(60));
    if fresh {
        return Ok(access_token);
    }
    // Expired but not refreshable (Notion/Slack long-lived tokens) — hand back
    // what we have; the API call will surface a real auth error if it's invalid.
    let Some(rt) = refresh_token else {
        return Ok(access_token);
    };

    // Which provider minted this? Recorded in the credential metadata at the
    // time of the OAuth callback.
    let provider_slug = cred
        .metadata
        .get("provider")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            DataSourceError::Permanent(format!(
                "credential {credential_id} metadata is missing `provider`; cannot refresh"
            ))
        })?;
    let prov = provider::lookup(provider_slug).ok_or_else(|| {
        DataSourceError::Permanent(format!("unknown oauth provider `{provider_slug}`"))
    })?;

    // Resolve client creds: per-tenant BYO (needs the run-time pool/crypto) +
    // operator env. Without the run-time handle, fall back to env only.
    let creds = match &ctx.oauth {
        Some(rt) => client::resolve_client(&rt.pool, &rt.crypto, ctx.tenant, prov)
            .await
            .map_err(|e| DataSourceError::Transient(format!("resolve oauth client: {e}")))?,
        None => client::env_client(prov),
    };
    let Some(creds) = creds else {
        return Err(DataSourceError::Transient(format!(
            "no OAuth client configured for `{provider_slug}`; cannot refresh token"
        )));
    };

    let tokens = flow::refresh_token(&ctx.http, prov, &creds, &rt)
        .await
        .map_err(|e| DataSourceError::Transient(format!("token refresh: {e}")))?;

    // Persist the rotated tokens (some providers rotate the refresh token too;
    // keep the old one if a new one wasn't returned).
    let new_value = CredentialValue::Oauth2 {
        access_token: tokens.access_token.clone(),
        refresh_token: tokens.refresh_token.or(Some(rt)),
        expires_at: tokens.expires_at,
    };
    ctx.credentials
        .update_value(ctx.tenant, credential_id, &new_value)
        .await
        .map_err(|e| DataSourceError::Transient(format!("persist refreshed token: {e}")))?;

    Ok(tokens.access_token)
}
