//! Integration tests for `/v1/auth/config` (SPA runtime auth discovery) and
//! the `/v1/auth/tokens` API-token CRUD (the CLI/MCP auth story when
//! Supabase is the primary login).
//!
//! Requires `--features kyma-server/test-support`. Each test spins up an
//! isolated Postgres container via testcontainers.

#![cfg(feature = "test-support")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_server::auth::{
    passwords::hash_password, AuthLayerState, EnvAuthBackend, Role, SessionAuthBackend,
};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

// Tower services are consumed by `oneshot`; build a fresh app per request
// (same pattern as auth_handler_it.rs).
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
    let backend: Arc<dyn kyma_server::auth::AuthBackend> = Arc::new(SessionAuthBackend::new(
        catalog.clone(),
        EnvAuthBackend::from_str(""),
        true,
    ));
    let login_router = kyma_server::auth_handler::auth_login_router(catalog.clone());
    let session_router = kyma_server::auth_handler::auth_session_router(catalog.clone()).layer(
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

async fn send(state: &kyma_server::QueryState, req: Request<Body>) -> axum::http::Response<Body> {
    build_auth_app(state).oneshot(req).await.unwrap()
}

async fn body_json(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn login(state: &kyma_server::QueryState, username: &str, password: &str) -> String {
    let body = serde_json::json!({ "username": username, "password": password });
    let req = Request::builder()
        .method("POST")
        .uri("/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = send(state, req).await;
    assert_eq!(resp.status(), StatusCode::OK, "login should succeed");
    body_json(resp).await["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn authed(method: &str, uri: &str, token: &str, body: Option<Value>) -> Request<Body> {
    let builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    match body {
        Some(v) => builder
            .body(Body::from(serde_json::to_string(&v).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    }
}

/// `GET /v1/auth/config` defaults to password mode (no Supabase env).
#[tokio::test]
async fn auth_config_defaults_to_password_provider() {
    let state = kyma_server::test_support::seeded_state_empty().await;

    let req = Request::builder()
        .uri("/v1/auth/config")
        .body(Body::empty())
        .unwrap();
    let resp = send(&state, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert_eq!(json["provider"], "password");
    assert!(json.get("supabase_url").is_none() || json["supabase_url"].is_null());
}

/// Full API-token lifecycle: mint → authenticate with it → list → revoke →
/// the token stops working. Role capping: a write user cannot mint admin.
#[tokio::test]
async fn api_token_lifecycle_and_role_capping() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let cat = &state.catalog;
    cat.create_user("dev", &hash_password("pw123").unwrap(), "write")
        .await
        .unwrap();

    let access = login(&state, "dev", "pw123").await;

    // ---- Mint (requesting admin must be capped to the caller's write) ----
    let resp = send(
        &state,
        authed(
            "POST",
            "/v1/auth/tokens",
            &access,
            Some(serde_json::json!({ "name": "ci-bot", "role": "admin" })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let minted = body_json(resp).await;
    let api_token = minted["token"].as_str().expect("raw token returned once");
    assert_eq!(minted["role"], "write", "requested admin capped to write");

    // ---- The minted token authenticates ----
    let resp = send(&state, authed("GET", "/v1/auth/me", api_token, None)).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // ---- List shows it (and an id we can revoke by) ----
    let resp = send(&state, authed("GET", "/v1/auth/tokens", &access, None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let listed = body_json(resp).await;
    let tokens = listed["tokens"].as_array().expect("tokens array");
    let entry = tokens
        .iter()
        .find(|t| t["name"] == "ci-bot")
        .expect("minted token listed");
    assert_eq!(entry["role"], "write");
    let id = entry["id"].as_str().expect("token id").to_string();

    // ---- Revoke ----
    let resp = send(
        &state,
        authed("DELETE", &format!("/v1/auth/tokens/{id}"), &access, None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // ---- The token no longer authenticates ----
    let resp = send(&state, authed("GET", "/v1/auth/me", api_token, None)).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // ---- Revoking again → 404 ----
    let resp = send(
        &state,
        authed("DELETE", &format!("/v1/auth/tokens/{id}"), &access, None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
