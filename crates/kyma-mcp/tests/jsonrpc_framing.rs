//! Wire-protocol edge cases for JSON-RPC 2.0 + Streamable HTTP.
//! Exercises the surface called out by Slice 1a verification: parse errors,
//! version mismatches, batch requests, notifications.

use kyma_mcp::{router, McpState, ServerInfo, ToolDispatch};
use kyma_server::agent::SharedToolCtx;
use kyma_server::test_support::seeded_state_empty;
use serde_json::{json, Value};

async fn build() -> String {
    let state = seeded_state_empty().await;
    let url = std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog,
        format: state.format,
        pool: Some(pool),
        memory: None,
        hitl: None,
    };
    let mcp_state = McpState {
        dispatch: ToolDispatch::new(shared),
        server_info: ServerInfo {
            name: "kyma".into(),
            version: "test".into(),
        },
    };
    let app = router(mcp_state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    format!("http://{addr}/mcp/v1")
}

#[tokio::test]
async fn parse_error_for_invalid_json() {
    let base = build().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&base)
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32700);
    assert!(resp["id"].is_null());
}

#[tokio::test]
async fn invalid_request_for_wrong_version() {
    let base = build().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&base)
        .json(&json!({"jsonrpc":"1.0","id":1,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32600);
}

#[tokio::test]
async fn method_not_found_for_unknown_method() {
    let base = build().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&base)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"resources/list"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32601);
}

#[tokio::test]
async fn invalid_params_for_tools_call_without_name() {
    let base = build().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&base)
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32602);
}

#[tokio::test]
async fn batch_with_mixed_results() {
    let base = build().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&base)
        .json(&json!([
            {"jsonrpc":"2.0","id":1,"method":"tools/list"},
            {"jsonrpc":"2.0","id":2,"method":"does/not/exist"}
        ]))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let arr = resp.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let by_id: std::collections::HashMap<i64, &Value> =
        arr.iter().map(|v| (v["id"].as_i64().unwrap(), v)).collect();
    assert!(by_id[&1]["result"]["tools"].is_array());
    assert_eq!(by_id[&2]["error"]["code"], -32601);
}

#[tokio::test]
async fn batch_of_only_notifications_returns_202() {
    let base = build().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(&base)
        .json(&json!([
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","method":"notifications/initialized"}
        ]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);
    let bytes = resp.bytes().await.unwrap();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn empty_batch_is_invalid_request() {
    let base = build().await;
    let client = reqwest::Client::new();
    let resp: Value = client
        .post(&base)
        .body("[]")
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32600);
}
