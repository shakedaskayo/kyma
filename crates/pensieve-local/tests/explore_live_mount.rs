//! Mount test for `GET /v1/explore/live` via the REAL app assembly.
//!
//! Exercises `build_local_app` — the same router-construction path that
//! `run_serve` uses — to guard against regressions where a route is missing or
//! an auth layer is incorrectly applied.
//!
//! Two assertions:
//!
//! 1. `GET /v1/explore/live` with NO auth + NO WebSocket upgrade headers →
//!    status 400 or 426 (NOT 401/403/404/405).  axum's `WebSocketUpgrade`
//!    extractor replies 400/426 to plain HTTP GETs; any other status would
//!    mean the route is absent (404/405) or hidden behind auth (401/403).
//!
//! 2. `GET /v1/dashboards` WITHOUT auth → 401.  Pins that the auth middleware
//!    IS still applied to protected read routes after the refactor.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_format_tlm::TelemetryFormat;
use kyma_local::build_local_app;
use kyma_server::auth::EnvAuthBackend;
use object_store::memory::InMemory;
use tower::ServiceExt as _;

/// Build the full local app over an in-memory catalog + store.
async fn local_app() -> axum::Router {
    let catalog: Arc<dyn Catalog> = Arc::new(
        SqliteCatalog::connect_in_memory()
            .await
            .expect("in-memory SQLite catalog"),
    );
    let store = Arc::new(InMemory::new());
    let format = Arc::new(TelemetryFormat::new(store, "kyma-test"));
    // Use a static-token backend with no tokens — auth is enabled but no
    // valid token is presented in the test requests, so protected routes
    // return 401.
    let backend: Arc<dyn kyma_server::auth::AuthBackend> =
        Arc::new(EnvAuthBackend::from_str("test-token:read"));
    let (app, _agent_state, _brain_state) = build_local_app(catalog, format, backend, None, None, None, None, None, kyma_local::watcher_status::LocalWatcherStatus::default(), None);
    app
}

/// `GET /v1/explore/live` without a WebSocket upgrade must return 400 or 426
/// (route mounted and public — axum's WS extractor rejects plain HTTP GETs),
/// NOT 401/403 (auth blocking it) or 404/405 (route absent).
#[tokio::test]
async fn explore_live_reachable_without_auth() {
    let app = local_app().await;

    let req = Request::builder()
        .method("GET")
        .uri("/v1/explore/live")
        .body(Body::empty())
        .unwrap();

    let status = app.oneshot(req).await.unwrap().status();

    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UPGRADE_REQUIRED,
        "expected 400 or 426 (route mounted, WS upgrade required), got {status}; \
         404/405 means the route is not mounted, 401/403 means auth middleware is \
         blocking the path"
    );
}

/// `GET /v1/data-sources/watchers` must be mounted on a fresh local server
/// (even with no cc-sync watcher running) and return `200 {"items":[]}` —
/// the web UI's watchers tab needs an empty state, not a 404. Behind read
/// auth like the rest of the API surface.
#[tokio::test]
async fn watchers_endpoint_returns_empty_items() {
    let app = local_app().await;

    let req = Request::builder()
        .method("GET")
        .uri("/v1/data-sources/watchers")
        .header("authorization", "Bearer test-token")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "watchers endpoint must be mounted unconditionally in local mode"
    );
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json, serde_json::json!({ "items": [] }));
}

/// `GET /v1/dashboards` without auth must return 401.
///
/// Guards against the inverse regression: if the auth middleware were stripped
/// from protected routes during refactoring, this would return 200 instead of
/// 401, and the test would catch it.
#[tokio::test]
async fn dashboards_requires_auth() {
    let app = local_app().await;

    let req = Request::builder()
        .method("GET")
        .uri("/v1/dashboards")
        .body(Body::empty())
        .unwrap();

    let status = app.oneshot(req).await.unwrap().status();

    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "GET /v1/dashboards without auth must return 401; \
         got {status} — auth middleware may have been stripped from the read router"
    );
}
