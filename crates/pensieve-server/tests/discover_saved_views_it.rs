//! Integration tests for the saved-views CRUD surface.
//!
//! Pattern: `tower::ServiceExt::oneshot`. The tests build an app that includes
//! both the read router (`GET /v1/explore/views`) and the write router
//! (`POST`/`PATCH`/`DELETE`), wrapped with auth middleware that injects a
//! `Principal` with a deterministic `subject`. The `EnvAuthBackend` always
//! emits `subject = None`, which the saved-views handlers reject with
//! `403 missing_subject`; the local `TestAuth` backend below fills in a stable
//! subject so the CRUD endpoints run end-to-end.

#![cfg(feature = "test-support")]

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use pensieve_server::auth::{AuthBackend, AuthError, AuthLayerState, Principal, Role};
use std::sync::Arc;
use tower::ServiceExt;

/// Minimal test-only auth backend: any non-empty Bearer becomes an Admin
/// principal in the default tenant with `subject = Some("test-user")`. This
/// lets the saved-views handlers (which gate on `principal.subject`) run
/// end-to-end without standing up the full session-backend stack.
struct TestAuth;

#[async_trait]
impl AuthBackend for TestAuth {
    fn enabled(&self) -> bool {
        true
    }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        if token.is_empty() {
            return Err(AuthError::MissingToken);
        }
        Ok(Principal {
            tenant: pensieve_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: Some("test-user".to_string()),
            allowed_databases: None,
            allowed_realms: None,
        })
    }
}

fn build_app(state: pensieve_server::QueryState) -> axum::Router {
    let backend: Arc<dyn AuthBackend> = Arc::new(TestAuth);
    let layer = AuthLayerState {
        backend,
        required: Role::Read,
    };

    let read_router = pensieve_server::router(state.clone());
    let write_router = pensieve_server::discover_views_write_router(
        state
            .pg_pool
            .clone()
            .expect("test QueryState is always built with a pg_pool"),
    );

    read_router
        .merge(write_router)
        .layer(axum::middleware::from_fn_with_state(
            layer,
            pensieve_server::auth::require_role_middleware,
        ))
}

async fn one(state: pensieve_server::QueryState, req: Request<Body>) -> (StatusCode, String) {
    let app = build_app(state);
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

fn auth_header(req: axum::http::request::Builder) -> axum::http::request::Builder {
    req.header("authorization", "Bearer test-token")
}

#[tokio::test]
async fn create_then_list_then_delete() {
    let state = pensieve_server::test_support::seeded_state_two_databases().await;

    // 1) Create
    let req = auth_header(Request::builder().method("POST").uri("/v1/explore/views"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"prod-only","sources":["obs.*"]}"#))
        .unwrap();
    let (status, text) = one(state.clone(), req).await;
    assert_eq!(status, StatusCode::CREATED, "create body: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let id = v["id"].as_str().unwrap().to_string();

    // 2) List
    let req = auth_header(Request::builder().method("GET").uri("/v1/explore/views"))
        .body(Body::empty())
        .unwrap();
    let (status, text) = one(state.clone(), req).await;
    assert_eq!(status, StatusCode::OK, "list body: {text}");
    let arr: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        arr.as_array()
            .unwrap()
            .iter()
            .any(|v| v["name"] == "prod-only"),
        "list missing prod-only: {text}"
    );

    // 3) Delete
    let req = auth_header(
        Request::builder()
            .method("DELETE")
            .uri(format!("/v1/explore/views/{id}")),
    )
    .body(Body::empty())
    .unwrap();
    let (status, text) = one(state.clone(), req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete body: {text}");

    // 4) List is now empty
    let req = auth_header(Request::builder().method("GET").uri("/v1/explore/views"))
        .body(Body::empty())
        .unwrap();
    let (status, text) = one(state, req).await;
    assert_eq!(status, StatusCode::OK, "list-after-delete body: {text}");
    let arr: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        arr.as_array().unwrap().len(),
        0,
        "list-after-delete not empty: {text}"
    );
}

#[tokio::test]
async fn create_with_duplicate_name_returns_409() {
    let state = pensieve_server::test_support::seeded_state_two_databases().await;
    let body = r#"{"name":"dup","sources":["obs.*"]}"#;

    let req = auth_header(Request::builder().method("POST").uri("/v1/explore/views"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let (s1, t1) = one(state.clone(), req).await;
    assert_eq!(s1, StatusCode::CREATED, "first create body: {t1}");

    let req = auth_header(Request::builder().method("POST").uri("/v1/explore/views"))
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let (s2, text) = one(state, req).await;
    assert_eq!(s2, StatusCode::CONFLICT, "body: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "name_conflict");
}

#[tokio::test]
async fn create_with_empty_name_returns_400() {
    let state = pensieve_server::test_support::seeded_state_two_databases().await;
    let req = auth_header(Request::builder().method("POST").uri("/v1/explore/views"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"","sources":["obs.*"]}"#))
        .unwrap();
    let (s, text) = one(state, req).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "body: {text}");
}

#[tokio::test]
async fn create_with_empty_sources_returns_400() {
    let state = pensieve_server::test_support::seeded_state_two_databases().await;
    let req = auth_header(Request::builder().method("POST").uri("/v1/explore/views"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"x","sources":[]}"#))
        .unwrap();
    let (s, text) = one(state, req).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "body: {text}");
}

#[tokio::test]
async fn search_with_view_scope_resolves_via_saved_view() {
    let state = pensieve_server::test_support::seeded_state_two_databases().await;

    // Create a view that scopes to obs.* only.
    let req = auth_header(Request::builder().method("POST").uri("/v1/explore/views"))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"obs-only","sources":["obs.*"]}"#))
        .unwrap();
    let (s, text) = one(state.clone(), req).await;
    assert_eq!(s, StatusCode::CREATED, "create body: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let view_id = v["id"].as_str().unwrap().to_string();

    // Use it in a search.
    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "view", "view_id": view_id },
    });
    let req = auth_header(
        Request::builder()
            .method("POST")
            .uri("/v1/explore/search"),
    )
    .header("content-type", "application/json")
    .body(Body::from(serde_json::to_string(&body).unwrap()))
    .unwrap();
    let (status, text) = one(state, req).await;
    assert_eq!(status, StatusCode::OK, "search body: {text}");

    // First line is the plan; sources should only include obs.*.
    let plan: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert_eq!(plan["type"], "plan", "first frame must be plan; body: {text}");
    let sources: Vec<&str> = plan["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    assert!(
        sources.iter().all(|s| s.starts_with("obs.")),
        "expected only obs.* sources, got: {sources:?}"
    );
}

#[tokio::test]
async fn search_with_missing_view_returns_404() {
    let state = pensieve_server::test_support::seeded_state_two_databases().await;
    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "view", "view_id": "00000000-0000-0000-0000-000000000000" },
    });
    let req = auth_header(
        Request::builder()
            .method("POST")
            .uri("/v1/explore/search"),
    )
    .header("content-type", "application/json")
    .body(Body::from(serde_json::to_string(&body).unwrap()))
    .unwrap();
    let (status, text) = one(state, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body: {text}");
}
