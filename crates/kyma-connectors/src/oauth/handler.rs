//! HTTP handlers for the OAuth2 authorization-code flow.
//!
//! Two routers: an authenticated one (`start`, `flows/:state` poll) mounted
//! behind the `Role::Write` layer, and an **unauthenticated** callback router
//! (the IdP redirect carries no bearer) mounted alongside the login route. The
//! callback's only trust anchor is the single-use `state` token stored in
//! `oauth_flows`, which also carries the initiating tenant.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::{Duration, Utc};
use kyma_catalog::PgCredentialStore;
use kyma_core::credentials::CredentialValue;
use kyma_core::crypto::Crypto;
use kyma_core::tenant::TenantId;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::{client, flow, provider, store};

/// Shared state for the OAuth routers.
#[derive(Clone)]
pub struct OAuthState {
    pub pool: PgPool,
    pub credentials: Arc<PgCredentialStore>,
    pub crypto: Arc<Crypto>,
    pub http: reqwest::Client,
    /// Externally reachable origin used to build `redirect_uri`
    /// (`{redirect_base}/v1/oauth/{provider}/callback`).
    pub redirect_base: String,
    /// Where the callback sends the browser back to (the web UI origin).
    pub ui_return_base: String,
}

impl OAuthState {
    /// Construct with a fresh internal HTTP client (so the binary doesn't need
    /// a direct `reqwest` dependency).
    pub fn new(
        pool: PgPool,
        credentials: Arc<PgCredentialStore>,
        crypto: Arc<Crypto>,
        redirect_base: String,
        ui_return_base: String,
    ) -> Self {
        Self {
            pool,
            credentials,
            crypto,
            http: reqwest::Client::new(),
            redirect_base,
            ui_return_base,
        }
    }
}

/// Authenticated routes: start a flow + poll its status. Wrap with the
/// `Role::Write` auth layer in the binary.
pub fn oauth_authed_router(state: OAuthState) -> Router {
    Router::new()
        .route("/v1/oauth/:provider/start", post(start))
        .route("/v1/oauth/flows/:state", get(flow_status))
        .with_state(state)
}

/// Unauthenticated callback route — mount like the login router (no auth layer).
pub fn oauth_callback_router(state: OAuthState) -> Router {
    Router::new()
        .route("/v1/oauth/:provider/callback", get(callback))
        .with_state(state)
}

// ── start ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StartReq {
    /// Connector this credential is for (e.g. `"googledrive"`). Stored on the
    /// flow + credential metadata for display.
    connector_type: String,
    /// Override the provider's default scopes.
    #[serde(default)]
    scopes: Option<Vec<String>>,
    /// Label for the credential the callback mints.
    #[serde(default)]
    label: Option<String>,
    /// Optional bring-your-own client app (persisted per-tenant, encrypted).
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<String>,
}

#[derive(Serialize)]
struct StartResp {
    authorize_url: String,
    state: String,
}

async fn start(
    Extension(tenant): Extension<TenantId>,
    State(s): State<OAuthState>,
    Path(provider_slug): Path<String>,
    Json(req): Json<StartReq>,
) -> Response {
    let Some(prov) = provider::lookup(&provider_slug) else {
        return err(StatusCode::BAD_REQUEST, format!("unknown provider `{provider_slug}`"));
    };

    // Persist bring-your-own client creds first so resolution picks them up.
    if let (Some(cid), Some(csec)) = (req.client_id.as_deref(), req.client_secret.as_deref()) {
        if !cid.is_empty() && !csec.is_empty() {
            if let Err(e) = client::upsert_byo(&s.pool, &s.crypto, tenant, prov.slug, cid, csec).await
            {
                return err(StatusCode::INTERNAL_SERVER_ERROR, format!("store client creds: {e}"));
            }
        }
    }

    let creds = match client::resolve_client(&s.pool, &s.crypto, tenant, prov).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return err(
                StatusCode::SERVICE_UNAVAILABLE,
                format!(
                    "no OAuth client configured for `{}` — set KYMA_OAUTH_{}_CLIENT_ID/_CLIENT_SECRET or provide your own",
                    prov.slug, prov.env_key
                ),
            );
        }
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("resolve client: {e}")),
    };

    // CSRF state + (optionally) PKCE.
    let state_tok = random_token(32);
    let (verifier, challenge) = if prov.use_pkce {
        let v = random_token(48);
        let c = pkce_challenge(&v);
        (Some(v), Some(c))
    } else {
        (None, None)
    };

    let scopes: Vec<String> = req
        .scopes
        .unwrap_or_else(|| prov.default_scopes.iter().map(|s| s.to_string()).collect());
    let scope_str = scopes.join(prov.scope_sep);
    let redirect_uri = format!(
        "{}/v1/oauth/{}/callback",
        s.redirect_base.trim_end_matches('/'),
        prov.slug
    );
    let label = req
        .label
        .filter(|l| !l.trim().is_empty())
        .unwrap_or_else(|| format!("{} ({})", prov.display, req.connector_type));

    let enc_verifier = match verifier.as_ref().map(|v| s.crypto.encrypt(v.as_bytes())).transpose() {
        Ok(v) => v,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("encrypt verifier: {e}")),
    };

    let new_flow = store::NewFlow {
        tenant,
        state: &state_tok,
        provider: prov.slug,
        connector_type: &req.connector_type,
        label: &label,
        scopes: &scope_str,
        redirect_uri: &redirect_uri,
        enc_code_verifier: enc_verifier,
        expires_at: Utc::now() + Duration::minutes(10),
    };
    if let Err(e) = store::insert_flow(&s.pool, &new_flow).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("persist flow: {e}"));
    }

    // Build the authorize URL.
    let mut url = match Url::parse(prov.authorize_url) {
        Ok(u) => u,
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("provider url: {e}")),
    };
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code");
        q.append_pair("client_id", &creds.client_id);
        q.append_pair("redirect_uri", &redirect_uri);
        if !scope_str.is_empty() {
            q.append_pair("scope", &scope_str);
        }
        q.append_pair("state", &state_tok);
        if let Some(c) = &challenge {
            q.append_pair("code_challenge", c);
            q.append_pair("code_challenge_method", "S256");
        }
        for (k, v) in prov.extra_authorize_params {
            q.append_pair(k, v);
        }
    }

    (
        StatusCode::OK,
        Json(StartResp {
            authorize_url: url.to_string(),
            state: state_tok,
        }),
    )
        .into_response()
}

// ── callback (unauthenticated) ────────────────────────────────────────────────

#[derive(Deserialize)]
struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

async fn callback(
    State(s): State<OAuthState>,
    Path(provider_slug): Path<String>,
    Query(q): Query<CallbackQuery>,
) -> Response {
    // Provider-reported error (user denied, etc.).
    if let Some(e) = q.error {
        if let Some(st) = &q.state {
            store::mark_error(&s.pool, st, &e).await;
        }
        return callback_html(&s.ui_return_base, &provider_slug, q.state.as_deref(), Outcome::Error(e));
    }

    let (Some(code), Some(state_tok)) = (q.code, q.state) else {
        return callback_html(
            &s.ui_return_base,
            &provider_slug,
            None,
            Outcome::Error("missing authorization code or state".into()),
        );
    };

    let Some(prov) = provider::lookup(&provider_slug) else {
        return callback_html(
            &s.ui_return_base,
            &provider_slug,
            Some(&state_tok),
            Outcome::Error(format!("unknown provider `{provider_slug}`")),
        );
    };

    let flow_row = match store::consume_flow(&s.pool, &state_tok).await {
        Ok(f) => f,
        Err(e) => {
            return callback_html(
                &s.ui_return_base,
                &provider_slug,
                Some(&state_tok),
                Outcome::Error(e.to_string()),
            );
        }
    };

    let creds = match client::resolve_client(&s.pool, &s.crypto, flow_row.tenant, prov).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            store::mark_error(&s.pool, &state_tok, "no client configured").await;
            return callback_html(
                &s.ui_return_base,
                &provider_slug,
                Some(&state_tok),
                Outcome::Error("no OAuth client configured".into()),
            );
        }
        Err(e) => {
            store::mark_error(&s.pool, &state_tok, &e.to_string()).await;
            return callback_html(
                &s.ui_return_base,
                &provider_slug,
                Some(&state_tok),
                Outcome::Error(e.to_string()),
            );
        }
    };

    let verifier = flow_row.enc_code_verifier.as_deref().and_then(|enc| {
        s.crypto
            .decrypt(enc)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    });

    let tokens = match flow::exchange_code(
        &s.http,
        prov,
        &creds,
        &code,
        &flow_row.redirect_uri,
        verifier.as_deref(),
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            store::mark_error(&s.pool, &state_tok, &e.to_string()).await;
            return callback_html(
                &s.ui_return_base,
                &provider_slug,
                Some(&state_tok),
                Outcome::Error(format!("token exchange failed: {e}")),
            );
        }
    };

    let value = CredentialValue::Oauth2 {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at: tokens.expires_at,
    };
    let metadata = serde_json::json!({
        "provider": prov.slug,
        "connector_type": flow_row.connector_type,
        "scopes": flow_row.scopes,
        "obtained_via": "oauth",
    });

    let summary = match s
        .credentials
        .create(flow_row.tenant, &flow_row.label, &value, metadata)
        .await
    {
        Ok(sm) => sm,
        Err(e) => {
            store::mark_error(&s.pool, &state_tok, &e.to_string()).await;
            return callback_html(
                &s.ui_return_base,
                &provider_slug,
                Some(&state_tok),
                Outcome::Error(format!("store credential failed: {e}")),
            );
        }
    };

    let _ = store::mark_completed(&s.pool, &state_tok, summary.id).await;
    callback_html(
        &s.ui_return_base,
        &provider_slug,
        Some(&state_tok),
        Outcome::Ok {
            credential_id: summary.id,
            label: summary.label,
        },
    )
}

// ── poll ──────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct FlowStatusResp {
    status: String,
    credential_id: Option<Uuid>,
}

async fn flow_status(
    Extension(tenant): Extension<TenantId>,
    State(s): State<OAuthState>,
    Path(state_tok): Path<String>,
) -> Response {
    match store::flow_status(&s.pool, tenant, &state_tok).await {
        Ok(Some((status, credential_id))) => {
            (StatusCode::OK, Json(FlowStatusResp { status, credential_id })).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (code, Json(serde_json::json!({ "error": msg.into() }))).into_response()
}

fn random_token(n_bytes: usize) -> String {
    use base64::Engine as _;
    let mut bytes = vec![0u8; n_bytes];
    for b in bytes.iter_mut() {
        *b = fastrand::u8(..);
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine as _;
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

enum Outcome {
    Ok { credential_id: Uuid, label: String },
    Error(String),
}

/// Tiny HTML page served to the popup/redirect target: it posts the result to
/// the opener (`postMessage`) and, as a fallback for full-redirect / Tauri,
/// navigates back to the web UI. The payload carries no secret (only the new
/// `credential_id` + label), so a `"*"` target is acceptable — the UI validates
/// `event.origin` + `state`.
fn callback_html(ui_base: &str, provider: &str, state: Option<&str>, outcome: Outcome) -> Response {
    let ui = ui_base.trim_end_matches('/');
    let (payload, redirect) = match &outcome {
        Outcome::Ok { credential_id, label } => (
            serde_json::json!({
                "type": "kyma-oauth",
                "ok": true,
                "provider": provider,
                "state": state,
                "credential_id": credential_id,
                "label": label,
            }),
            format!("{ui}/connectors/new?provider={provider}&credential_id={credential_id}&oauth=ok"),
        ),
        Outcome::Error(msg) => (
            serde_json::json!({
                "type": "kyma-oauth",
                "ok": false,
                "provider": provider,
                "state": state,
                "error": msg,
            }),
            format!("{ui}/connectors/new?provider={provider}&oauth=error"),
        ),
    };
    // Embed as JS; `</` is escaped so the JSON can't break out of the <script>.
    let payload_js = payload.to_string().replace("</", "<\\/");
    let html = format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>Connecting…</title></head>
<body style="font-family:system-ui,sans-serif;padding:2rem;text-align:center;color:#334155">
<p>Finishing authorization — you can close this window.</p>
<script>
(function() {{
  var msg = {payload_js};
  try {{ if (window.opener) {{ window.opener.postMessage(msg, "*"); }} }} catch (e) {{}}
  try {{ window.close(); }} catch (e) {{}}
  setTimeout(function() {{ window.location.replace({redirect:?}); }}, 400);
}})();
</script>
</body></html>"#,
        payload_js = payload_js,
        redirect = redirect,
    );
    (StatusCode::OK, Html(html)).into_response()
}
