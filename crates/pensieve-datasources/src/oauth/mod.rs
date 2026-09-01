//! OAuth2 authorization-code framework for data sources.
//!
//! Data sources that authenticate with `auth_mode: "oauth"` (Notion, Google,
//! Atlassian, Slack, …) obtain a [`pensieve_core::credentials::CredentialValue::Oauth2`]
//! credential through the browser flow implemented here:
//!
//! 1. `POST /v1/oauth/:provider/start` — the UI asks for an authorize URL. We
//!    mint a single-use `state` (+ a PKCE verifier where the provider supports
//!    it), persist it in `oauth_flows` stamped with the caller's tenant, and
//!    return the provider's authorize URL.
//! 2. The user authorizes; the provider redirects to
//!    `GET /v1/oauth/:provider/callback` (unauthenticated — a cross-site
//!    redirect carries no bearer). We consume the `state` row, exchange the
//!    code for tokens, store them as an encrypted `Oauth2` credential, and hand
//!    the new `credential_id` back to the opener via `postMessage` + redirect.
//! 3. Data sources resolve a fresh access token at run time via
//!    [`token::valid_access_token`], which refreshes + persists in place.
//!
//! Client app credentials (`client_id` / `client_secret`) come from operator
//! env (`PENSIEVE_OAUTH_<PROVIDER>_CLIENT_ID` / `_CLIENT_SECRET`) or a per-tenant
//! bring-your-own row in `oauth_clients`; see [`client::resolve_client`].

pub mod client;
pub mod flow;
pub mod handler;
pub mod provider;
pub mod store;
pub mod token;

use std::sync::Arc;

pub use handler::{oauth_authed_router, oauth_callback_router, OAuthState};
pub use provider::{provider_for_data_source, scopes_for_data_source, OAuthProvider};
pub use token::valid_access_token;

/// Run-time OAuth capability handed to data sources through
/// [`crate::types::DataSourceCtx::oauth`]. Lets [`token::valid_access_token`]
/// resolve a provider's client credentials (operator env **and** the per-tenant
/// `oauth_clients` table) and decrypt them when refreshing an expired token.
///
/// Optional: when absent, refresh falls back to operator-env client creds only.
#[derive(Clone)]
pub struct OAuthRuntime {
    pub pool: sqlx::PgPool,
    pub crypto: Arc<pensieve_core::crypto::Crypto>,
}
