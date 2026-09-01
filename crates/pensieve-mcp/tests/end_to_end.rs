//! End-to-end MCP handshake test against a real TCP listener.

use pensieve_mcp::{router, McpState, ServerInfo, ToolDispatch};
use pensieve_server::agent::SharedToolCtx;
use pensieve_server::auth::{
    require_role_middleware, AuthBackend, AuthLayerState, EnvAuthBackend, Role,
};
use pensieve_server::test_support::seeded_state_with_obs_otel_logs;
use serde_json::{json, Value};
use std::sync::Arc;

#[tokio::test]
async fn full_mcp_handshake_against_seeded_server() {
    let state = seeded_state_with_obs_otel_logs().await;
    let url = std::env::var("PENSIEVE_TEST_DATABASE_URL").expect("PENSIEVE_TEST_DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let shared = SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: Some(pool),
        memory: None,
        hitl: None,
        memory_settings_path: None,
    };
    let mcp_state = McpState {
        dispatch: ToolDispatch::new(shared),
        builder: None,
        server_info: ServerInfo {
            name: "pensieve".into(),
            version: "test".into(),
        },
    };
    let backend: Arc<dyn AuthBackend> = Arc::new(EnvAuthBackend::from_str("mcp-token:read"));
    let app = router(mcp_state).layer(axum::middleware::from_fn_with_state(
        AuthLayerState {
            backend,
            required: Role::Read,
        },
        require_role_middleware,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{addr}/mcp/v1");

    // 1. initialize
    let init: Value = client
        .post(&base)
        .bearer_auth("mcp-token")
        .json(&json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-03-26","capabilities":{},
                      "clientInfo":{"name":"test","version":"0"}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");

    // 2. notifications/initialized
    let resp = client
        .post(&base)
        .bearer_auth("mcp-token")
        .json(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::ACCEPTED);

    // 3. tools/list
    let list: Value = client
        .post(&base)
        .bearer_auth("mcp-token")
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 30);
    assert!(
        tools
            .iter()
            .any(|t| t["name"].as_str() == Some("graph_analytics")),
        "graph_analytics should be exposed"
    );

    // 4. tools/call list_databases
    let call: Value = client
        .post(&base)
        .bearer_auth("mcp-token")
        .json(&json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"list_databases","arguments":{}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(call["result"]["isError"], false);
    let dbs = call["result"]["structuredContent"]["databases"]
        .as_array()
        .unwrap();
    assert!(dbs.iter().any(|v| v.as_str() == Some("obs")));

    // 5. tools/call graph_analytics — no graph registered in `obs`, so the tool
    //    dispatches + resolves and returns a structured "graph not found" (proves
    //    the wiring end-to-end without needing an ingested graph).
    let ga: Value = client
        .post(&base)
        .bearer_auth("mcp-token")
        .json(&json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"graph_analytics",
                      "arguments":{"database":"obs","graph":"nope","kind":"pagerank"}}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ga_err = ga["result"]["structuredContent"]["error"]
        .as_str()
        .unwrap_or_default();
    assert!(ga_err.contains("graph not found"), "got: {ga:?}");
}

#[tokio::test]
async fn rejects_request_without_bearer_token() {
    let state = seeded_state_with_obs_otel_logs().await;
    let url = std::env::var("PENSIEVE_TEST_DATABASE_URL").expect("PENSIEVE_TEST_DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    let shared = SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: state.catalog,
        format: state.format,
        pool: Some(pool),
        memory: None,
        hitl: None,
        memory_settings_path: None,
    };
    let mcp_state = McpState {
        dispatch: ToolDispatch::new(shared),
        builder: None,
        server_info: ServerInfo {
            name: "pensieve".into(),
            version: "test".into(),
        },
    };
    let backend: Arc<dyn AuthBackend> = Arc::new(EnvAuthBackend::from_str("mcp-token:read"));
    let app = router(mcp_state).layer(axum::middleware::from_fn_with_state(
        AuthLayerState {
            backend,
            required: Role::Read,
        },
        require_role_middleware,
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });

    let resp = reqwest::Client::new()
        .post(&format!("http://{addr}/mcp/v1"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
}
