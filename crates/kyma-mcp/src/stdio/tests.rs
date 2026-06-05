//! stdio-transport framing tests. These drive the real read→dispatch→write
//! loop over in-memory byte streams — no Postgres, no sockets. The context is
//! built from the embedded SQLite catalog + an in-memory object store + a lazy
//! (never-connected) Postgres pool, which is sufficient for `initialize`,
//! `tools/list`, notifications, and parse errors (none touch the DB).

use crate::initialize::ServerInfo;
use crate::stdio::serve;
use crate::tools::ToolDispatch;
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_format_tlm::TelemetryFormat;
use kyma_server::agent::SharedToolCtx;
use kyma_storage::{build_object_store, StorageConfig};
use serde_json::Value;
use std::sync::Arc;

async fn db_free_dispatch() -> (ToolDispatch, ServerInfo) {
    let store = build_object_store(&StorageConfig::Memory).unwrap();
    let format = Arc::new(TelemetryFormat::new(store, "local"));
    let catalog: Arc<dyn Catalog> = Arc::new(
        SqliteCatalog::connect_in_memory()
            .await
            .expect("sqlite catalog"),
    );
    // Local mode: no Postgres pool at all.
    let shared = SharedToolCtx {
        catalog,
        format,
        pool: None,
        memory: None,
    };
    (
        ToolDispatch::new(shared),
        ServerInfo {
            name: "kyma".into(),
            version: "test".into(),
        },
    )
}

/// Run a set of newline-delimited request lines through the loop, returning the
/// parsed response lines (in order).
async fn run(lines: &[&str]) -> Vec<Value> {
    let (dispatch, info) = db_free_dispatch().await;
    let input = format!("{}\n", lines.join("\n"));
    let mut output: Vec<u8> = Vec::new();
    serve(&dispatch, &info, input.as_bytes(), &mut output)
        .await
        .expect("serve loop");
    String::from_utf8(output)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str::<Value>(l).expect("each output line is JSON"))
        .collect()
}

#[tokio::test]
async fn initialize_then_tools_list_over_stdio() {
    let out = run(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    ])
    .await;

    // The notification yields no line → exactly two responses.
    assert_eq!(out.len(), 2, "notification produced no response line");

    assert_eq!(out[0]["id"], 1);
    assert_eq!(out[0]["result"]["protocolVersion"], "2025-03-26");

    assert_eq!(out[1]["id"], 2);
    let tools = out[1]["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    // The full context-engine toolset is exposed over stdio (query + memory).
    assert!(
        names.contains(&"memory_search"),
        "memory_search present: {names:?}"
    );
    assert!(names.contains(&"save_memory"));
    assert!(names.contains(&"run_kql"));
    assert!(names.contains(&"graph_traverse"));
    assert!(
        tools.len() >= 15,
        "expected the full toolset, got {}",
        tools.len()
    );
}

#[tokio::test]
async fn malformed_line_yields_parse_error() {
    let out = run(&["this is not json"]).await;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["id"], Value::Null);
    assert_eq!(out[0]["error"]["code"], -32700); // ParseError
}

#[tokio::test]
async fn unknown_method_yields_method_not_found() {
    let out = run(&[r#"{"jsonrpc":"2.0","id":7,"method":"does/not/exist"}"#]).await;
    assert_eq!(out.len(), 1);
    assert_eq!(out[0]["id"], 7);
    assert_eq!(out[0]["error"]["code"], -32601); // MethodNotFound
}

#[tokio::test]
async fn batch_request_returns_array() {
    let out = run(&[
        r#"[{"jsonrpc":"2.0","id":1,"method":"tools/list"},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":2,"method":"tools/list"}]"#,
    ])
    .await;
    // One output line containing a JSON array of the two non-notification responses.
    assert_eq!(out.len(), 1);
    let arr = out[0].as_array().expect("batch response is an array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[1]["id"], 2);
}
