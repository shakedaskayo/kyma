//! HTTP handlers for the login / refresh / me / logout auth surface.
//!
//! Sessions use short-lived **access** tokens + long-lived **refresh** tokens,
//! both grouped by a `session_id`. The client authenticates API calls with the
//! access token and silently exchanges the refresh token for a new pair when it
//! nears expiry (refresh-token rotation). Logout revokes the whole session.
//! Refresh tokens are barred from authenticating ordinary API requests.
//!
//! # Routes
//!
//! ## Unauthenticated (mount via [`auth_login_router`])
//! - `POST /v1/auth/login` — username + password → access+refresh pair.
//! - `POST /v1/auth/refresh` — refresh token → rotated access+refresh pair.
//!
//! ## Authenticated (mount via [`auth_session_router`], wrapped with
//!   `require_role_middleware` at `Role::Read`)
//! - `GET /v1/auth/me` — return the current principal.
//! - `POST /v1/auth/logout` — revoke the whole session.

use crate::auth::{hash_token, Principal};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use chrono::{Duration, Utc};
use kyma_core::catalog::Catalog;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// -------------------------------------------------------------------------
// State
// -------------------------------------------------------------------------

/// Shared handler state — just the catalog.
#[derive(Clone)]
pub struct AuthState {
    pub catalog: Arc<dyn Catalog>,
}

// -------------------------------------------------------------------------
// Request / response shapes
// -------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Response for both login and refresh: a fresh access+refresh token pair.
#[derive(Debug, Serialize)]
pub struct TokenPairResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: chrono::DateTime<Utc>,
    pub refresh_expires_at: chrono::DateTime<Utc>,
    pub user: UserInfo,
}

/// Access-token lifetime. Short by design — the client silently refreshes.
/// Overridable via `KYMA_ACCESS_TTL_SECS` (default 3600 = 1h).
fn access_ttl() -> Duration {
    std::env::var("KYMA_ACCESS_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .map(Duration::seconds)
        .unwrap_or_else(|| Duration::hours(1))
}

/// Refresh-token lifetime (the effective max session age before re-login).
/// Overridable via `KYMA_REFRESH_TTL_SECS` (default 2592000 = 30d).
fn refresh_ttl() -> Duration {
    std::env::var("KYMA_REFRESH_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .map(Duration::seconds)
        .unwrap_or_else(|| Duration::days(30))
}

/// Mint a random URL-safe token; returns `(raw_token, sha256_hash)`.
fn mint_token() -> (String, Vec<u8>) {
    let mut raw_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_bytes);
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw_bytes);
    let hash = hash_token(&raw);
    (raw, hash)
}

/// Issue + persist an access+refresh pair for a login session.
async fn issue_token_pair(
    catalog: &dyn Catalog,
    session_id: uuid::Uuid,
    role: &str,
    subject: &str,
) -> std::result::Result<TokenPairResponse, String> {
    let now = Utc::now();
    let access_expires_at = now + access_ttl();
    let refresh_expires_at = now + refresh_ttl();

    let (access_raw, access_hash) = mint_token();
    let (refresh_raw, refresh_hash) = mint_token();

    catalog
        .insert_session_token(&access_hash, role, Some(subject), "access", access_expires_at, session_id)
        .await
        .map_err(|e| e.to_string())?;
    catalog
        .insert_session_token(&refresh_hash, role, Some(subject), "refresh", refresh_expires_at, session_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(TokenPairResponse {
        access_token: access_raw,
        refresh_token: refresh_raw,
        access_expires_at,
        refresh_expires_at,
        user: UserInfo { username: subject.to_string(), role: role.to_string() },
    })
}

#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub username: Option<String>,
    pub role: String,
}

#[derive(Debug, Serialize)]
pub struct UserInfo {
    pub username: String,
    pub role: String,
}

// -------------------------------------------------------------------------
// Error helpers
// -------------------------------------------------------------------------

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": {
                "code": "unauthorized",
                "message": "invalid credentials"
            }
        })),
    )
        .into_response()
}

fn internal_error_response(msg: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({
            "error": {
                "code": "internal",
                "message": msg
            }
        })),
    )
        .into_response()
}

// -------------------------------------------------------------------------
// Handlers
// -------------------------------------------------------------------------

/// `POST /v1/auth/login` — exchange credentials for a session token.
async fn login_handler(
    State(state): State<AuthState>,
    Json(body): Json<LoginRequest>,
) -> Response {
    // Look up the user (returns hash for verification).
    let result = state
        .catalog
        .get_user_with_hash(&body.username)
        .await;

    let (user, stored_hash) = match result {
        Ok(Some(pair)) => pair,
        Ok(None) => return unauthorized_response(),
        Err(e) => return internal_error_response(&e.to_string()),
    };

    // Verify password with argon2.
    if !crate::auth::passwords::verify_password(&body.password, &stored_hash) {
        return unauthorized_response();
    }

    // Mint a fresh login session: an access + refresh token pair grouped by a
    // new session_id so logout/refresh can act on the whole session.
    let session_id = uuid::Uuid::new_v4();
    match issue_token_pair(state.catalog.as_ref(), session_id, &user.role, &user.username).await {
        Ok(pair) => (StatusCode::OK, Json(pair)).into_response(),
        Err(e) => internal_error_response(&e),
    }
}

/// `POST /v1/auth/refresh` — exchange a valid refresh token for a rotated pair.
///
/// Unauthenticated route: the refresh token itself is the credential. On
/// success the presented refresh token is revoked (rotation) and a new
/// access+refresh pair is issued within the same session.
async fn refresh_handler(
    State(state): State<AuthState>,
    Json(body): Json<RefreshRequest>,
) -> Response {
    let presented_hash = hash_token(body.refresh_token.trim());

    let claim = match state.catalog.lookup_refresh_token(&presented_hash).await {
        Ok(Some(c)) => c,
        Ok(None) => return unauthorized_response(),
        Err(e) => return internal_error_response(&e.to_string()),
    };

    // Rotate: revoke the presented refresh token so it can't be replayed.
    if let Err(e) = state.catalog.revoke_api_token(&presented_hash).await {
        return internal_error_response(&e.to_string());
    }

    let subject = claim.subject.clone().unwrap_or_default();
    match issue_token_pair(state.catalog.as_ref(), claim.session_id, &claim.role, &subject).await {
        Ok(pair) => (StatusCode::OK, Json(pair)).into_response(),
        Err(e) => internal_error_response(&e),
    }
}

/// `GET /v1/auth/me` — return the current principal (injected by middleware).
async fn me_handler(Extension(principal): Extension<Principal>) -> Response {
    let role = format!("{:?}", principal.role).to_lowercase();
    (
        StatusCode::OK,
        Json(MeResponse {
            username: principal.subject,
            role,
        }),
    )
        .into_response()
}

/// `POST /v1/auth/logout` — revoke the presented session token.
async fn logout_handler(State(state): State<AuthState>, req: Request) -> Response {
    let raw_token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim);

    let Some(token) = raw_token else {
        return (StatusCode::BAD_REQUEST, "missing Authorization header").into_response();
    };

    // Revoke the whole session (this access token + its paired refresh token),
    // so a stolen refresh token can't outlive an explicit logout.
    let hash = hash_token(token);
    match state.catalog.revoke_session_by_token(&hash).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => internal_error_response(&e.to_string()),
    }
}

// -------------------------------------------------------------------------
// API tokens (long-lived, for CLI / MCP / CI — the non-browser auth story)
// -------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateTokenRequest {
    /// Display label, surfaced as `name` in listings.
    pub name: Option<String>,
    /// Requested role — silently capped at the caller's own role.
    pub role: Option<String>,
    /// Optional expiry in days (default: no expiry).
    pub expires_days: Option<i64>,
}

/// `POST /v1/auth/tokens` — mint a long-lived API token (`kind='api'`).
///
/// The raw token is returned exactly once; only its SHA-256 is stored. The
/// granted role is `min(requested, caller's role)` so nobody escalates via
/// token minting.
async fn create_api_token_handler(
    State(state): State<AuthState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<CreateTokenRequest>,
) -> Response {
    use crate::auth::Role;
    let requested = body
        .role
        .as_deref()
        .and_then(Role::parse)
        .unwrap_or(principal.role);
    let granted = requested.min(principal.role);
    let role_str = format!("{granted:?}").to_lowercase();

    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| principal.subject.clone());
    let expires_at = body.expires_days.map(|d| Utc::now() + Duration::days(d));

    let (raw, hash) = mint_token();
    if let Err(e) = state
        .catalog
        .insert_api_token(&hash, &role_str, name.as_deref(), "api", expires_at)
        .await
    {
        return internal_error_response(&e.to_string());
    }
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "token": raw,
            "id": hex::encode(&hash),
            "name": name,
            "role": role_str,
            "expires_at": expires_at,
        })),
    )
        .into_response()
}

/// `GET /v1/auth/tokens` — list API tokens (`kind='api'`). Raw tokens are
/// unrecoverable; `id` is the hex of the stored hash, usable with DELETE.
async fn list_api_tokens_handler(State(state): State<AuthState>) -> Response {
    match state.catalog.list_api_tokens("api").await {
        Ok(tokens) => {
            let items: Vec<_> = tokens
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "id": hex::encode(&t.token_hash),
                        "name": t.subject,
                        "role": t.role,
                        "created_at": t.created_at,
                        "last_used_at": t.last_used_at,
                        "expires_at": t.expires_at,
                        "revoked": t.revoked,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({ "tokens": items }))).into_response()
        }
        Err(e) => internal_error_response(&e.to_string()),
    }
}

/// `DELETE /v1/auth/tokens/{id}` — revoke an API token by its hash hex.
async fn revoke_api_token_handler(
    State(state): State<AuthState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let Ok(hash) = hex::decode(&id) else {
        return (StatusCode::BAD_REQUEST, "invalid token id").into_response();
    };
    match state.catalog.revoke_api_token(&hash).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => internal_error_response(&e.to_string()),
    }
}

// -------------------------------------------------------------------------
// Runtime auth discovery for the SPA
// -------------------------------------------------------------------------

/// `GET /v1/auth/config` — unauthenticated. Tells the web app which login
/// flow to render: kyma's password form, or Supabase (with the runtime
/// project URL + anon key, so one build works against any deployment).
async fn auth_config_handler() -> Response {
    let supabase_mode = std::env::var("KYMA_AUTH_BACKEND")
        .map(|v| v == "supabase")
        .unwrap_or(false);
    let body = match (
        supabase_mode,
        std::env::var("KYMA_SUPABASE_URL").ok().filter(|s| !s.is_empty()),
    ) {
        (true, Some(url)) => {
            let providers: Vec<String> = std::env::var("KYMA_SUPABASE_PROVIDERS")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            serde_json::json!({
                "provider": "supabase",
                "supabase_url": url.trim_end_matches('/'),
                "supabase_anon_key": std::env::var("KYMA_SUPABASE_ANON_KEY").ok(),
                "oauth_providers": providers,
            })
        }
        _ => serde_json::json!({ "provider": "password" }),
    };
    (StatusCode::OK, Json(body)).into_response()
}

// -------------------------------------------------------------------------
// First-run setup: signup + status + environment probe
// -------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SignupRequest {
    pub username: String,
    pub password: String,
}

/// `POST /v1/auth/signup` — create the FIRST admin user (first-run setup).
///
/// Unauthenticated, but only succeeds while no users exist; once setup is
/// complete it returns 409 so this can't be used to mint extra admins. On
/// success it issues a token pair (auto-login) so the wizard proceeds signed in.
async fn signup_handler(State(state): State<AuthState>, Json(body): Json<SignupRequest>) -> Response {
    let username = body.username.trim().to_string();
    if username.is_empty() || body.password.len() < 8 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": {"code": "invalid",
                "message": "username is required and password must be at least 8 characters"}})),
        )
            .into_response();
    }
    match state.catalog.count_users().await {
        Ok(0) => {}
        Ok(_) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": {"code": "already_setup",
                    "message": "setup is already complete — sign in instead"}})),
            )
                .into_response();
        }
        Err(e) => return internal_error_response(&e.to_string()),
    }
    let phc = match crate::auth::passwords::hash_password(&body.password) {
        Ok(h) => h,
        Err(e) => return internal_error_response(&format!("hashing password: {e}")),
    };
    if let Err(e) = state.catalog.create_user(&username, &phc, "admin").await {
        return internal_error_response(&e.to_string());
    }
    let session_id = uuid::Uuid::new_v4();
    match issue_token_pair(state.catalog.as_ref(), session_id, "admin", &username).await {
        Ok(pair) => (StatusCode::CREATED, Json(pair)).into_response(),
        Err(e) => internal_error_response(&e),
    }
}

/// `GET /v1/auth/status` — unauthenticated. Lets the web app decide between
/// showing the first-run setup wizard vs. the login screen on load.
async fn auth_status_handler(State(state): State<AuthState>) -> Response {
    let users_exist = matches!(state.catalog.count_users().await, Ok(n) if n > 0);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "users_exist": users_exist,
            "setup_required": !users_exist,
        })),
    )
        .into_response()
}

/// `GET /v1/setup/probe` — unauthenticated host capability probe for the setup
/// wizard's AI-engine step: which engines/keys are available, whether Ollama is
/// running and `gemma4:latest` is installed, plus a recommended default.
async fn env_probe_handler() -> Response {
    let anthropic_key_present =
        std::env::var("ANTHROPIC_API_KEY").map(|v| !v.is_empty()).unwrap_or(false);
    let openai_key_present =
        std::env::var("OPENAI_API_KEY").map(|v| !v.is_empty()).unwrap_or(false);
    let claude_binary_found = crate::agent::engine::claude_cli::locate_binary().is_some();
    let (ollama_reachable, ollama_models) = probe_ollama().await;
    let gemma4_present = ollama_models.iter().any(|m| m.starts_with("gemma4"));

    // On-device gemma4 is the preferred default (private, no key). Fall back to
    // whatever cloud capability is actually available.
    let recommend = if gemma4_present {
        serde_json::json!({"kind": "ollama", "model": "gemma4:latest"})
    } else if anthropic_key_present {
        serde_json::json!({"kind": "anthropic", "model": "claude-sonnet-4-6"})
    } else if claude_binary_found {
        serde_json::json!({"kind": "claude_cli", "model": "sonnet"})
    } else if openai_key_present {
        serde_json::json!({"kind": "openai", "model": "gpt-4o"})
    } else if ollama_reachable && !ollama_models.is_empty() {
        serde_json::json!({"kind": "ollama", "model": ollama_models[0]})
    } else {
        // Nothing detected — still default to on-device gemma4 and hint a pull.
        serde_json::json!({"kind": "ollama", "model": "gemma4:latest", "needs_pull": true})
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ollama_reachable": ollama_reachable,
            "ollama_models": ollama_models,
            "gemma4_present": gemma4_present,
            "anthropic_key_present": anthropic_key_present,
            "openai_key_present": openai_key_present,
            "claude_binary_found": claude_binary_found,
            "recommend": recommend,
        })),
    )
        .into_response()
}

/// Probe the local Ollama daemon for installed models. Returns
/// `(reachable, model_names)`. Short timeout so the wizard stays snappy.
async fn probe_ollama() -> (bool, Vec<String>) {
    let host = std::env::var("KYMA_OLLAMA_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://localhost:11434".to_string());
    let url = format!("{}/api/tags", host.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(1500))
        .build()
    {
        Ok(c) => c,
        Err(_) => return (false, Vec::new()),
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let models = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("models").cloned())
                .and_then(|m| m.as_array().cloned())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (true, models)
        }
        _ => (false, Vec::new()),
    }
}

// -------------------------------------------------------------------------
// Router builders
// -------------------------------------------------------------------------

/// Build the **unauthenticated** login router.
///
/// Mount this with `.merge(auth_login_router(...))` BEFORE the auth layer —
/// it must NOT be wrapped with `require_role_middleware`.
pub fn auth_login_router(catalog: Arc<dyn Catalog>) -> Router {
    let state = AuthState { catalog };
    Router::new()
        .route("/v1/auth/login", post(login_handler))
        .route("/v1/auth/refresh", post(refresh_handler))
        .route("/v1/auth/signup", post(signup_handler))
        .route("/v1/auth/status", get(auth_status_handler))
        .route("/v1/auth/config", get(auth_config_handler))
        .route("/v1/setup/probe", get(env_probe_handler))
        .with_state(state)
}

/// Build the **authenticated** session router (me + logout).
///
/// The caller MUST wrap this router with
/// `require_role_middleware(Role::Read)` so the bearer token is validated and
/// a [`Principal`] is inserted into request extensions before these handlers
/// are reached.
pub fn auth_session_router(catalog: Arc<dyn Catalog>) -> Router {
    let state = AuthState { catalog };
    Router::new()
        .route("/v1/auth/me", get(me_handler))
        .route("/v1/auth/logout", post(logout_handler))
        .route(
            "/v1/auth/tokens",
            get(list_api_tokens_handler).post(create_api_token_handler),
        )
        .route(
            "/v1/auth/tokens/:id",
            axum::routing::delete(revoke_api_token_handler),
        )
        .with_state(state)
}
