//! Integration tests for the auth handler (login / me / logout).
//!
//! Requires `--features kyma-server/test-support`.
//! Each test spins up an isolated Postgres container via testcontainers.

#![cfg(feature = "test-support")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_server::auth::{
    passwords::hash_password, AuthLayerState, EnvAuthBackend, Role, SessionAuthBackend,
};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

// -------------------------------------------------------------------------
// Helper: build a combined login+session app
// -------------------------------------------------------------------------

fn build_auth_app(
    state: &kyma_server::QueryState,
) -> impl tower::Service<
    Request<Body>,
    Response = axum::http::Response<Body>,
    Error = std::convert::Infallible,
    Future = impl std::future::Future<
        Output = Result<axum::http::Response<Body>, std::convert::Infallible>,
    >,
> {
    let catalog = state.catalog.clone();

    // The session backend: checks catalog tokens + env fallback.
    let backend: Arc<dyn kyma_server::auth::AuthBackend> = Arc::new(
        SessionAuthBackend::new(catalog.clone(), EnvAuthBackend::from_str(""), true),
    );

    // Unauthenticated login route.
    let login_router = kyma_server::auth_handler::auth_login_router(catalog.clone());

    // Authenticated me/logout routes.
    let session_router =
        kyma_server::auth_handler::auth_session_router(catalog.clone()).layer(
            axum::middleware::from_fn_with_state(
                AuthLayerState {
                    backend,
                    required: Role::Read,
                },
                kyma_server::auth::require_role_middleware,
            ),
        );

    login_router.merge(session_router)
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

/// (a) Login succeeds → returns a token; (b) /me with that token returns the
/// user; (c) logout → 204, then /me with the same token → 401; (d) login
/// with wrong password → 401.
#[tokio::test]
async fn auth_handler_full_flow() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let cat = &state.catalog;

    // Seed an admin user.
    let phc = hash_password("pw123").unwrap();
    cat.create_user("admin", &phc, "admin").await.unwrap();

    let app = build_auth_app(&state);

    // ---- (a) Successful login ----
    let login_body = serde_json::json!({ "username": "admin", "password": "pw123" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&login_body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "login should succeed");

    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap();
    // Login returns an access + refresh token pair (auth_handler::TokenPair); the
    // access token authenticates ordinary API calls. User identity is asserted via
    // /me below — the pair carries no user body.
    let token = body["access_token"]
        .as_str()
        .expect("access_token in login response")
        .to_owned();
    assert!(!token.is_empty(), "access token must be non-empty");
    assert!(
        body["refresh_token"]
            .as_str()
            .is_some_and(|t| !t.is_empty()),
        "refresh token must be present and non-empty"
    );

    // Re-build app for each oneshot call (tower services are consumed).
    let app = build_auth_app(&state);

    // ---- (b) GET /v1/auth/me with the issued token ----
    let req = Request::builder()
        .method("GET")
        .uri("/v1/auth/me")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "/me should return 200");

    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["username"].as_str().unwrap(), "admin");
    assert_eq!(body["role"].as_str().unwrap(), "admin");

    // ---- (c) Logout → 204; then /me → 401 ----
    let app = build_auth_app(&state);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/auth/logout")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT, "logout should return 204");

    // After revocation, /me must reject the same token.
    let app = build_auth_app(&state);
    let req = Request::builder()
        .method("GET")
        .uri("/v1/auth/me")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "/me with revoked token should return 401"
    );

    // ---- (d) Login with wrong password → 401 ----
    let app = build_auth_app(&state);
    let bad_login = serde_json::json!({ "username": "admin", "password": "wrongpw" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&bad_login).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "wrong password should return 401"
    );
}

/// Login for a non-existent user → 401 (no user enumeration).
#[tokio::test]
async fn login_nonexistent_user_returns_401() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let app = build_auth_app(&state);

    let body = serde_json::json!({ "username": "nobody", "password": "any" });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
