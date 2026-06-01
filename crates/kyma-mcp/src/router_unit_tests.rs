//! Unit tests for the POST /mcp/v1 channel using tower's oneshot.

use crate::initialize::ServerInfo;
use crate::router::{router, McpState};
use crate::tools::ToolDispatch;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use kyma_server::agent::SharedToolCtx;
use kyma_server::test_support::seeded_state_empty;
use serde_json::{json, Value};
use tower::util::ServiceExt;

async fn build_app() -> axum::Router {
    let state = seeded_state_empty().await;
    let pool = sqlx::PgPool::connect(
        &std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL"),
    )
    .await
    .unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog,
        format: state.format,
        pool,
    };
    router(McpState {
        dispatch: ToolDispatch::new(shared),
        server_info: ServerInfo {
            name: "kyma".into(),
            version: "test".into(),
        },
    })
}

async fn jsonrpc(app: axum::Router, body: Value) -> Value {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp/v1")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn initialize_round_trip() {
    let app = build_app().await;
    let resp = jsonrpc(
        app,
        json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}
        }),
    )
    .await;
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
}

#[tokio::test]
async fn tools_list_returns_all() {
    let app = build_app().await;
    let resp = jsonrpc(
        app,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 13);
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let app = build_app().await;
    let resp = jsonrpc(
        app,
        json!({"jsonrpc":"2.0","id":3,"method":"does/not/exist"}),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32601);
}

#[tokio::test]
async fn malformed_json_returns_parse_error_with_null_id() {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp/v1")
        .header("content-type", "application/json")
        .body(Body::from("{not json"))
        .unwrap();
    let app = build_app().await;
    let resp = app.oneshot(req).await.unwrap();
    let bytes = to_bytes(resp.into_body(), 1_000_000).await.unwrap();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["code"], -32700);
    assert!(v["id"].is_null());
}

#[tokio::test]
async fn notifications_initialized_returns_no_body() {
    let req = Request::builder()
        .method("POST")
        .uri("/mcp/v1")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        ))
        .unwrap();
    let app = build_app().await;
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::ACCEPTED);
    let bytes = to_bytes(resp.into_body(), 1_000).await.unwrap();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn batch_request_returns_array() {
    let app = build_app().await;
    let body = json!([
        {"jsonrpc":"2.0","id":1,"method":"tools/list"},
        {"jsonrpc":"2.0","id":2,"method":"initialize",
         "params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}
    ]);
    let resp = jsonrpc(app, body).await;
    let arr = resp.as_array().expect("batch responds with array");
    assert_eq!(arr.len(), 2);
}
