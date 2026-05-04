//! End-to-end MCP handshake test against a real TCP listener.

use kyma_mcp::{router, McpState, ServerInfo, ToolDispatch};
use kyma_server::agent::SharedToolCtx;
use kyma_server::auth::{require_role_middleware, AuthConfig, Role};
use kyma_server::test_support::seeded_state_with_obs_otel_logs;
use serde_json::{json, Value};

#[tokio::test]
async fn full_mcp_handshake_against_seeded_server() {
    let state = seeded_state_with_obs_otel_logs().await;
    let url = std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool,
    };
    let mcp_state = McpState {
        dispatch: ToolDispatch::new(shared),
        server_info: ServerInfo { name: "kyma".into(), version: "test".into() },
    };
    let auth = AuthConfig::from_str("mcp-token:read");
    let app = router(mcp_state).layer(axum::middleware::from_fn_with_state(
        (auth, Role::Read),
        require_role_middleware,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.ok(); });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}/mcp/v1");

    // 1. initialize
    let init: Value = client.post(&base).bearer_auth("mcp-token")
        .json(&json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-03-26","capabilities":{},
                      "clientInfo":{"name":"test","version":"0"}}
        }))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");

    // 2. notifications/initialized
    let resp = client.post(&base).bearer_auth("mcp-token")
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);

    // 3. tools/list
    let list: Value = client.post(&base).bearer_auth("mcp-token")
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send().await.unwrap().json().await.unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 8);

    // 4. tools/call list_databases
    let call: Value = client.post(&base).bearer_auth("mcp-token")
        .json(&json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"list_databases","arguments":{}}
        }))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(call["result"]["isError"], false);
    let dbs = call["result"]["structuredContent"]["databases"].as_array().unwrap();
    assert!(dbs.iter().any(|v| v.as_str() == Some("obs")));
}

#[tokio::test]
async fn rejects_request_without_bearer_token() {
    let state = seeded_state_with_obs_otel_logs().await;
    let url = std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog,
        format: state.format,
        pool,
    };
    let mcp_state = McpState {
        dispatch: ToolDispatch::new(shared),
        server_info: ServerInfo { name: "kyma".into(), version: "test".into() },
    };
    let auth = AuthConfig::from_str("mcp-token:read");
    let app = router(mcp_state).layer(axum::middleware::from_fn_with_state(
        (auth, Role::Read),
        require_role_middleware,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.ok(); });

    let resp = reqwest::Client::new()
        .post(&format!("http://{addr}/mcp/v1"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}
