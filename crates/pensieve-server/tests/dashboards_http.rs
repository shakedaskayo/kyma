//! Integration tests for the `/v1/dashboards` CRUD endpoints.
//!
//! Requires `--features kyma-server/test-support` to compile.
//! Each test spins up its own isolated Postgres container via testcontainers.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

// -------------------------------------------------------------------------
// Test-app builder helpers
// -------------------------------------------------------------------------

/// Build a full test app: read routes wrapped with Role::Read, write routes
/// wrapped with Role::Write. Auth is configured with:
///   - `test-read-token:read`
///   - `test-write-token:write`
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
    let backend: std::sync::Arc<dyn kyma_server::auth::AuthBackend> = std::sync::Arc::new(
        kyma_server::auth::EnvAuthBackend::from_str(
            "test-read-token:read,test-write-token:write",
        ),
    );

    let read_router =
        kyma_server::router(state.clone()).layer(axum::middleware::from_fn_with_state(
            kyma_server::auth::AuthLayerState {
                backend: backend.clone(),
                required: kyma_server::auth::Role::Read,
            },
            kyma_server::auth::require_role_middleware,
        ));

    let write_router = kyma_server::dashboards_write_router(state.catalog.clone()).layer(
        axum::middleware::from_fn_with_state(
            kyma_server::auth::AuthLayerState {
                backend,
                required: kyma_server::auth::Role::Write,
            },
            kyma_server::auth::require_role_middleware,
        ),
    );

    read_router.merge(write_router)
}

/// Helper: POST /v1/dashboards with write token.
async fn post_dashboard<S>(app: &mut S, body: Value) -> axum::http::Response<Body>
where
    S: tower::Service<
        Request<Body>,
        Response = axum::http::Response<Body>,
        Error = std::convert::Infallible,
    >,
{
    let req = Request::builder()
        .method("POST")
        .uri("/v1/dashboards")
        .header("content-type", "application/json")
        .header("authorization", "Bearer test-write-token")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    app.call(req).await.unwrap()
}

/// Helper: GET /v1/dashboards/:id with read token.
async fn get_dashboard<S>(app: &mut S, id: &str) -> axum::http::Response<Body>
where
    S: tower::Service<
        Request<Body>,
        Response = axum::http::Response<Body>,
        Error = std::convert::Infallible,
    >,
{
    let req = Request::builder()
        .uri(format!("/v1/dashboards/{id}"))
        .header("authorization", "Bearer test-read-token")
        .body(Body::empty())
        .unwrap();
    app.call(req).await.unwrap()
}

/// Helper: GET /v1/dashboards with read token.
async fn list_dashboards<S>(app: &mut S) -> axum::http::Response<Body>
where
    S: tower::Service<
        Request<Body>,
        Response = axum::http::Response<Body>,
        Error = std::convert::Infallible,
    >,
{
    let req = Request::builder()
        .uri("/v1/dashboards")
        .header("authorization", "Bearer test-read-token")
        .body(Body::empty())
        .unwrap();
    app.call(req).await.unwrap()
}

/// Helper: PATCH /v1/dashboards/:id with write token.
async fn patch_dashboard<S>(app: &mut S, id: &str, body: Value) -> axum::http::Response<Body>
where
    S: tower::Service<
        Request<Body>,
        Response = axum::http::Response<Body>,
        Error = std::convert::Infallible,
    >,
{
    let req = Request::builder()
        .method("PATCH")
        .uri(format!("/v1/dashboards/{id}"))
        .header("content-type", "application/json")
        .header("authorization", "Bearer test-write-token")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    app.call(req).await.unwrap()
}

/// Helper: DELETE /v1/dashboards/:id with write token.
async fn delete_dashboard<S>(app: &mut S, id: &str) -> axum::http::Response<Body>
where
    S: tower::Service<
        Request<Body>,
        Response = axum::http::Response<Body>,
        Error = std::convert::Infallible,
    >,
{
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/v1/dashboards/{id}"))
        .header("authorization", "Bearer test-write-token")
        .body(Body::empty())
        .unwrap();
    app.call(req).await.unwrap()
}

async fn body_json(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

/// POST dashboard with no panels, then GET it back — shape is correct.
#[tokio::test]
async fn post_no_panels_then_get() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let mut app = authed_app(state);

    let resp = post_dashboard(
        &mut app,
        json!({ "name": "My Dashboard", "description": "smoke test" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    let id = v["id"].as_str().expect("id field");
    assert_eq!(v["name"], "My Dashboard");
    assert_eq!(v["description"], "smoke test");
    assert_eq!(v["panels"], json!([]));

    // GET it back.
    let resp2 = get_dashboard(&mut app, id).await;
    assert_eq!(resp2.status(), StatusCode::OK);
    let v2 = body_json(resp2).await;
    assert_eq!(v2["id"], v["id"]);
    assert_eq!(v2["name"], "My Dashboard");
    assert_eq!(v2["panels"], json!([]));
}

/// POST dashboard with 3 panels → all 3 come back with display_order preserved.
#[tokio::test]
async fn post_with_panels_preserves_display_order() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let mut app = authed_app(state);

    let resp = post_dashboard(
        &mut app,
        json!({
            "name": "Panel Dashboard",
            "panels": [
                { "title": "Chart", "panel_type": "chart", "config": {}, "grid_x": 0, "grid_y": 0, "grid_w": 6, "grid_h": 4, "display_order": 0 },
                { "title": "Table", "panel_type": "table", "config": {}, "grid_x": 6, "grid_y": 0, "grid_w": 6, "grid_h": 4, "display_order": 1 },
                { "title": "Stat",  "panel_type": "stat",  "config": {}, "grid_x": 0, "grid_y": 4, "grid_w": 3, "grid_h": 2, "display_order": 2 }
            ]
        }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED, "create failed");
    let v = body_json(resp).await;
    let panels = v["panels"].as_array().unwrap();
    assert_eq!(panels.len(), 3);
    assert_eq!(panels[0]["title"], "Chart");
    assert_eq!(panels[0]["display_order"], 0);
    assert_eq!(panels[1]["title"], "Table");
    assert_eq!(panels[1]["display_order"], 1);
    assert_eq!(panels[2]["title"], "Stat");
    assert_eq!(panels[2]["display_order"], 2);
}

/// PATCH name only → updated; panels are untouched.
#[tokio::test]
async fn patch_name_only_leaves_panels() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let mut app = authed_app(state);

    // Create with 1 panel.
    let create_resp = post_dashboard(
        &mut app,
        json!({
            "name": "Original",
            "panels": [
                { "title": "Panel A", "panel_type": "markdown", "config": {}, "grid_x": 0, "grid_y": 0, "grid_w": 12, "grid_h": 3, "display_order": 0 }
            ]
        }),
    )
    .await;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let v = body_json(create_resp).await;
    let id = v["id"].as_str().unwrap().to_owned();
    let panel_id = v["panels"][0]["id"].as_str().unwrap().to_owned();

    // PATCH name only.
    let patch_resp = patch_dashboard(&mut app, &id, json!({ "name": "Renamed" })).await;
    assert_eq!(patch_resp.status(), StatusCode::OK);
    let pv = body_json(patch_resp).await;
    assert_eq!(pv["name"], "Renamed");
    let panels_after = pv["panels"].as_array().unwrap();
    assert_eq!(panels_after.len(), 1, "panel count must be unchanged");
    assert_eq!(panels_after[0]["id"], panel_id, "same panel id");
}

/// PATCH with `panels: [...]` → atomic replace; old panel IDs are gone.
#[tokio::test]
async fn patch_panels_atomic_replace() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let mut app = authed_app(state);

    // Create with 2 panels.
    let create_resp = post_dashboard(
        &mut app,
        json!({
            "name": "Replaceable",
            "panels": [
                { "title": "Old A", "panel_type": "chart", "config": {}, "grid_x": 0, "grid_y": 0, "grid_w": 6, "grid_h": 4, "display_order": 0 },
                { "title": "Old B", "panel_type": "stat",  "config": {}, "grid_x": 6, "grid_y": 0, "grid_w": 6, "grid_h": 4, "display_order": 1 }
            ]
        }),
    )
    .await;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let v = body_json(create_resp).await;
    let id = v["id"].as_str().unwrap().to_owned();
    let old_id_a = v["panels"][0]["id"].as_str().unwrap().to_owned();
    let old_id_b = v["panels"][1]["id"].as_str().unwrap().to_owned();

    // PATCH with completely new panels.
    let patch_resp = patch_dashboard(
        &mut app,
        &id,
        json!({
            "panels": [
                { "title": "New Only", "panel_type": "markdown", "config": {}, "grid_x": 0, "grid_y": 0, "grid_w": 12, "grid_h": 6, "display_order": 0 }
            ]
        }),
    )
    .await;
    assert_eq!(patch_resp.status(), StatusCode::OK);
    let pv = body_json(patch_resp).await;
    let panels_after = pv["panels"].as_array().unwrap();
    assert_eq!(panels_after.len(), 1, "exactly 1 panel after replace");
    assert_eq!(panels_after[0]["title"], "New Only");

    // Confirm old panel ids are not present.
    let new_panel_id = panels_after[0]["id"].as_str().unwrap();
    assert_ne!(new_panel_id, old_id_a, "old panel A must be gone");
    assert_ne!(new_panel_id, old_id_b, "old panel B must be gone");
}

/// DELETE → 204; GET same id → 404.
#[tokio::test]
async fn delete_returns_204_then_get_404() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let mut app = authed_app(state);

    let create_resp = post_dashboard(&mut app, json!({ "name": "To Delete" })).await;
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let v = body_json(create_resp).await;
    let id = v["id"].as_str().unwrap().to_owned();

    let del_resp = delete_dashboard(&mut app, &id).await;
    assert_eq!(del_resp.status(), StatusCode::NO_CONTENT);

    let get_resp = get_dashboard(&mut app, &id).await;
    assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
}

/// Missing auth on write endpoints → 401.
#[tokio::test]
async fn missing_auth_returns_401_on_write() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let backend: std::sync::Arc<dyn kyma_server::auth::AuthBackend> = std::sync::Arc::new(
        kyma_server::auth::EnvAuthBackend::from_str("test-write-token:write"),
    );

    // Only wrap write router with auth — GET would be read-gated normally.
    let write_router = kyma_server::dashboards_write_router(state.catalog.clone()).layer(
        axum::middleware::from_fn_with_state(
            kyma_server::auth::AuthLayerState {
                backend,
                required: kyma_server::auth::Role::Write,
            },
            kyma_server::auth::require_role_middleware,
        ),
    );

    // POST without any token.
    let req = Request::builder()
        .method("POST")
        .uri("/v1/dashboards")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"name":"x"}"#))
        .unwrap();
    let resp = write_router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Unknown dashboard id on GET → 404.
#[tokio::test]
async fn get_unknown_id_returns_404() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let mut app = authed_app(state);

    let fake_id = "00000000-0000-0000-0000-000000000001";
    let resp = get_dashboard(&mut app, fake_id).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// List dashboards returns all created dashboards.
#[tokio::test]
async fn list_returns_all_dashboards() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let mut app = authed_app(state);

    post_dashboard(&mut app, json!({ "name": "Alpha" })).await;
    post_dashboard(&mut app, json!({ "name": "Beta" })).await;

    let resp = list_dashboards(&mut app).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // ordered by updated_at DESC, both are fresh so either order is fine;
    // just check both names are present.
    let names: Vec<&str> = arr.iter().map(|d| d["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Alpha"), "Alpha missing from list");
    assert!(names.contains(&"Beta"), "Beta missing from list");
}
