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
        .with_state(state)
}
