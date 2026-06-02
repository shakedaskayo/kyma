//! End-to-end integration tests for `POST /v1/explore/search`.
//!
//! Pattern: `tower::ServiceExt::oneshot` against a router with auth-disabled
//! `EnvAuthBackend` (which short-circuits to admin in `DEFAULT_TENANT`). The
//! seeded fixture provides two databases, schemas only — assertions are on
//! the NDJSON frame sequence, not row data. Each test spins up its own fresh
//! Postgres container via testcontainers.

#![cfg(feature = "test-support")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_server::auth::{AuthLayerState, EnvAuthBackend, Role};
use std::sync::Arc;
use tower::ServiceExt;

/// Serializes the tests that depend on `KYMA_DISCOVER_MAX_SOURCES`. cargo runs
/// the tests in this binary on parallel threads sharing one process env, so the
/// `set_var`/`remove_var` window in `scope_too_large_returns_400` would
/// otherwise leak the cap=1 into the tests that assume the default cap. Holding
/// this lock for the duration of each such test makes them mutually exclusive.
/// Poison-tolerant: a real assertion failure in one test must not mask the rest.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_guard() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Build a router with auth middleware in "disabled" mode — the middleware
/// will short-circuit to an Admin principal in `DEFAULT_TENANT`. This is the
/// same trick the auth handler integration tests use to bypass real tokens.
fn build_app(state: kyma_server::QueryState) -> axum::Router {
    let backend: Arc<dyn kyma_server::auth::AuthBackend> = Arc::new(EnvAuthBackend::from_str(""));
    kyma_server::router(state).layer(axum::middleware::from_fn_with_state(
        AuthLayerState {
            backend,
            required: Role::Read,
        },
        kyma_server::auth::require_role_middleware,
    ))
}

/// POST a JSON body to `/v1/explore/search` and drain the NDJSON stream
/// into a single buffer. Returns `(status, body_text)`.
async fn post_search(
    state: kyma_server::QueryState,
    body: serde_json::Value,
) -> (StatusCode, String) {
    let app = build_app(state);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/explore/search")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 16 * 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

/// Parse an NDJSON response body into a vector of frame JSON values.
fn frames(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("frame parse"))
        .collect()
}

#[tokio::test]
async fn search_emits_plan_first_done_last() {
    let _env = env_guard();
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    let body = serde_json::json!({ "query": "", "scope": { "kind": "all" } });
    let (status, text) = post_search(state, body).await;

    assert_eq!(status, StatusCode::OK, "body: {text}");
    let f = frames(&text);
    assert!(!f.is_empty(), "expected at least one frame; body: {text}");
    assert_eq!(
        f.first().unwrap()["type"],
        "plan",
        "first frame must be plan; body: {text}"
    );
    assert_eq!(
        f.last().unwrap()["type"],
        "done",
        "last frame must be done; body: {text}"
    );
}

#[tokio::test]
async fn search_plan_includes_both_seeded_sources() {
    let _env = env_guard();
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    let body = serde_json::json!({ "query": "", "scope": { "kind": "all" } });
    let (_, text) = post_search(state, body).await;
    let f = frames(&text);
    let plan_sources: Vec<&str> = f[0]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    assert!(
        plan_sources.contains(&"obs.otel_logs"),
        "plan must include obs.otel_logs (got: {plan_sources:?})"
    );
    assert!(
        plan_sources.contains(&"stg.http_reqs"),
        "plan must include stg.http_reqs (got: {plan_sources:?})"
    );
}

#[tokio::test]
async fn search_emits_one_source_done_per_source() {
    let _env = env_guard();
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    let body = serde_json::json!({ "query": "", "scope": { "kind": "all" } });
    let (_, text) = post_search(state, body).await;
    let f = frames(&text);
    let done_sources: Vec<&str> = f
        .iter()
        .filter(|fr| fr["type"] == "source_done")
        .map(|fr| fr["source"].as_str().unwrap())
        .collect();
    assert!(
        done_sources.contains(&"obs.otel_logs"),
        "missing source_done for obs.otel_logs; body: {text}"
    );
    assert!(
        done_sources.contains(&"stg.http_reqs"),
        "missing source_done for stg.http_reqs; body: {text}"
    );
    assert_eq!(
        done_sources.len(),
        2,
        "exactly one source_done per source; body: {text}"
    );
}

#[tokio::test]
async fn scope_restricted_to_one_db_pattern() {
    let _env = env_guard();
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "sources", "sources": ["obs.*"] },
    });
    let (_, text) = post_search(state, body).await;
    let f = frames(&text);
    let plan_sources: Vec<&str> = f[0]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["source"].as_str().unwrap())
        .collect();
    assert!(
        plan_sources.contains(&"obs.otel_logs"),
        "plan must include obs.otel_logs (got: {plan_sources:?})"
    );
    assert!(
        !plan_sources.contains(&"stg.http_reqs"),
        "stg.* should be excluded by scope `obs.*` (got: {plan_sources:?})"
    );
}

#[tokio::test]
async fn scope_too_large_returns_400() {
    let _env = env_guard();
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    // Force the cap below 2 so the two-source fixture trips the limit.
    std::env::set_var("KYMA_DISCOVER_MAX_SOURCES", "1");
    let body = serde_json::json!({ "query": "", "scope": { "kind": "all" } });
    let (status, text) = post_search(state, body).await;
    std::env::remove_var("KYMA_DISCOVER_MAX_SOURCES");

    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "scope_too_large");
}

#[tokio::test]
async fn bad_json_returns_400() {
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    let app = build_app(state);
    let req = Request::builder()
        .method("POST")
        .uri("/v1/explore/search")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn invalid_time_range_returns_400() {
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "all" },
        "time_range": { "from": "2026-05-28T13:00:00Z", "to": "2026-05-28T12:00:00Z" },
    });
    let (status, text) = post_search(state, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "bad_time_range");
}

#[tokio::test]
async fn view_not_found_returns_404() {
    let state = kyma_server::test_support::seeded_state_two_databases().await;
    let body = serde_json::json!({
        "query": "",
        "scope": { "kind": "view", "view_id": "00000000-0000-0000-0000-000000000000" },
    });
    let (status, _) = post_search(state, body).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
