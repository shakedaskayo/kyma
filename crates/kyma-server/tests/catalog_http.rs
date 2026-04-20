//! Integration tests for `GET /v1/catalog/schema`.
//!
//! These spin up a real Postgres (via testcontainers), seed an "obs" database
//! with an "otel_logs" table, and exercise the HTTP handler end-to-end through
//! axum's `oneshot` helper.
//!
//! Auth is configured directly via `AuthConfig::from_str` (no env-var
//! mutation needed) so the tests are hermetic.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

/// Build the test router with auth middleware using `test-read-token → read`.
fn authed_app(
    state: kyma_server::QueryState,
) -> impl tower::Service<
    axum::http::Request<axum::body::Body>,
    Response = axum::http::Response<axum::body::Body>,
    Error = std::convert::Infallible,
    Future = impl std::future::Future<
        Output = Result<axum::http::Response<axum::body::Body>, std::convert::Infallible>,
    >,
> {
    let auth = kyma_server::auth::AuthConfig::from_str("test-read-token:read");
    kyma_server::router(state).layer(axum::middleware::from_fn_with_state(
        (auth, kyma_server::auth::Role::Read),
        kyma_server::auth::require_role_middleware,
    ))
}

#[tokio::test]
async fn returns_schema_tree() {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let app = authed_app(state);

    let req = Request::builder()
        .uri("/v1/catalog/schema")
        .header("authorization", "Bearer test-read-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let v: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["databases"][0]["name"], "obs");
    assert_eq!(v["databases"][0]["tables"][0]["name"], "otel_logs");
    assert!(v["databases"][0]["tables"][0]["columns"].is_array());
}

#[tokio::test]
async fn requires_read_role() {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let app = authed_app(state);

    let req = Request::builder()
        .uri("/v1/catalog/schema")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
