//! HTTP handlers for the login / me / logout auth surface.
//!
//! # Routes
//!
//! ## Unauthenticated (mount via [`auth_login_router`])
//! - `POST /v1/auth/login` — username + password → session token.
//!
//! ## Authenticated (mount via [`auth_session_router`], wrapped with
//!   `require_role_middleware` at `Role::Read`)
//! - `GET /v1/auth/me` — return the current principal.
//! - `POST /v1/auth/logout` — revoke the session token.

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

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserInfo,
    pub expires_at: chrono::DateTime<Utc>,
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

    // Generate a 32-byte random session token.
    let mut raw_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_bytes);
    // base64url-encode (no padding) for a URL-safe, human-copyable token.
    use base64::Engine as _;
    let raw_token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw_bytes);

    // Store the SHA-256 hash.
    let token_hash = hash_token(&raw_token);
    let expires_at = Utc::now() + Duration::days(30);

    if let Err(e) = state
        .catalog
        .insert_api_token(
            &token_hash,
            &user.role,
            Some(&user.username),
            "session",
            Some(expires_at),
        )
        .await
    {
        return internal_error_response(&e.to_string());
    }

    (
        StatusCode::OK,
        Json(LoginResponse {
            token: raw_token,
            user: UserInfo {
                username: user.username,
                role: user.role,
            },
            expires_at,
        }),
    )
        .into_response()
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

    let hash = hash_token(token);
    match state.catalog.revoke_api_token(&hash).await {
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
