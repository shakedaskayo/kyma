# Cloud Slice 1a — MCP Server Crate + Claude Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a new `kyma-mcp` crate that exposes the engine's eight existing agent tools as a JSON-RPC 2.0 MCP server over Streamable HTTP at `/mcp/v1`, reusing the existing `EnvAuthBackend` bearer-token middleware, plus a separate `kyma-claude-skill` repo containing a SKILL.md manifest and README so Claude Code users can install the connector with one command.

**Architecture:** `kyma-mcp` is a thin JSON-RPC adapter — zero duplicated tool logic. It depends on `kyma-server` (path-dep), reuses the existing `tool_*` factories from `crates/kyma-server/src/agent/tools.rs`, builds each into an `Arc<dyn adk_rust::Tool>` once at construction, and dispatches `tools/call` requests by name to `tool.execute(SimpleToolContext::new("kyma-mcp"), args).await`. The crate exports an axum router (`POST /mcp/v1` for JSON-RPC, `GET /mcp/v1` for the SSE upgrade defined by Streamable HTTP), which `kyma-bin` mounts behind the existing `require_role_middleware(Role::Read)` layer. Slice 2 swaps `EnvAuthBackend` for `DbAuthBackend`; nothing in this crate changes when that happens.

**Tech Stack:** Rust 1.95, axum 0.7 (`POST`/`GET`/SSE), tokio 1, serde + serde_json, async-trait, adk-rust 0.6 (`Tool` trait + `SimpleToolContext`), `kyma-server::agent::tools::*` factories. New crate `kyma-mcp`. Separate repo `kyma-claude-skill` (Markdown only). Spec: `docs/superpowers/specs/2026-05-02-kyma-cloud-platform-design.md`. MCP wire spec: https://modelcontextprotocol.io/.

---

## File Structure

**New files (Rust crate):**

- `crates/kyma-mcp/Cargo.toml` — manifest; deps `kyma-server` (path), `adk-rust`, `axum`, `tokio`, `serde`, `serde_json`, `async-trait`, `tracing`, `futures`, `tower-http`, `uuid`. Dev-deps: `tokio` full, `tower` util, `kyma-server` with `test-support` feature, `testcontainers`, `testcontainers-modules`.
- `crates/kyma-mcp/src/lib.rs` — crate root.
- `crates/kyma-mcp/src/jsonrpc.rs` — JSON-RPC 2.0 frame types.
- `crates/kyma-mcp/src/initialize.rs` — handles `initialize` method.
- `crates/kyma-mcp/src/tools.rs` — `ToolDispatch` over the eight `tool_*` factories.
- `crates/kyma-mcp/src/router.rs` — axum router exposing `POST /mcp/v1` + `GET /mcp/v1`.
- `crates/kyma-mcp/tests/jsonrpc_framing.rs` — wire-protocol edge-case tests.
- `crates/kyma-mcp/tests/end_to_end.rs` — full HTTP handshake test.

**Modified files:**

- `Cargo.toml` (workspace root) — add `crates/kyma-mcp` to `[workspace] members`; add `kyma-mcp = { path = "crates/kyma-mcp" }` to `[workspace.dependencies]`.
- `crates/kyma-server/src/agent/mod.rs` — re-export `tools::SharedToolCtx` and the eight `tool_*` factory functions.
- `crates/kyma-server/src/test_support.rs` — set `KYMA_TEST_DATABASE_URL` env var for external test crates.
- `crates/kyma-bin/src/main.rs` — build `mcp_state`, mount `kyma_mcp::router(mcp_state)` wrapped with `Role::Read` middleware.
- `crates/kyma-bin/Cargo.toml` — add `kyma-mcp = { workspace = true }` dep.

**New files (separate repo, NOT in kyma monorepo):**

- `kyma-claude-skill/SKILL.md` — manifest declaring an MCP server connector with a workspace URL placeholder + bearer-token instructions.
- `kyma-claude-skill/README.md` — user-facing setup guide.
- `kyma-claude-skill/.gitignore` — minimal Markdown-repo gitignore.
- `kyma-claude-skill/LICENSE` — Apache-2.0.

---

## Task 1: Workspace scaffolding for `kyma-mcp`

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/kyma-mcp/Cargo.toml`
- Create: `crates/kyma-mcp/src/lib.rs`

- [ ] **Step 1: Add the new crate to workspace members**

In root `Cargo.toml`, after `"crates/kyma-embed",` insert `"crates/kyma-mcp",`.

- [ ] **Step 2: Add to workspace deps**

After the `kyma-embed = { path = "crates/kyma-embed" }` line in `[workspace.dependencies]`, add:

```toml
kyma-mcp              = { path = "crates/kyma-mcp" }
```

- [ ] **Step 3: Create crate manifest**

Create `crates/kyma-mcp/Cargo.toml`:

```toml
[package]
name = "kyma-mcp"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "JSON-RPC 2.0 Model Context Protocol server over Streamable HTTP."

[lints]
workspace = true

[dependencies]
kyma-server = { workspace = true }
adk-rust = { workspace = true }
axum = { workspace = true }
tokio = { workspace = true }
tower-http = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
async-trait = { workspace = true }
futures = { workspace = true }
tracing = { workspace = true }
uuid = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["full"] }
tower = { workspace = true, features = ["util"] }
kyma-server = { workspace = true, features = ["test-support"] }
testcontainers = { workspace = true }
testcontainers-modules = { workspace = true }
sqlx = { workspace = true }
reqwest = { workspace = true }
```

- [ ] **Step 4: Create empty crate root**

Create `crates/kyma-mcp/src/lib.rs`:

```rust
//! JSON-RPC 2.0 Model Context Protocol server for kyma.
//!
//! Wraps the eight agent tools from `kyma_server::agent::tools` as MCP
//! tools, served over Streamable HTTP at `/mcp/v1`. Wire spec:
//! <https://modelcontextprotocol.io/>.

#![forbid(unsafe_code)]

pub mod initialize;
pub mod jsonrpc;
pub mod router;
pub mod tools;

pub use initialize::ServerInfo;
pub use router::{router, McpState};
pub use tools::ToolDispatch;
```

- [ ] **Step 5: Verify workspace compiles**

Run: `cargo check -p kyma-mcp`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/kyma-mcp/Cargo.toml crates/kyma-mcp/src/lib.rs
git commit -m "feat(kyma-mcp): scaffold new crate for JSON-RPC MCP server"
```

---

## Task 2: JSON-RPC 2.0 frame types

**Files:**
- Create: `crates/kyma-mcp/src/jsonrpc.rs`
- Create: `crates/kyma-mcp/src/jsonrpc_unit_tests.rs`

- [ ] **Step 1: Write failing test**

Append to `crates/kyma-mcp/src/lib.rs` (above the `pub use`):

```rust
#[cfg(test)]
mod jsonrpc_unit_tests;
```

Create `crates/kyma-mcp/src/jsonrpc_unit_tests.rs`:

```rust
use crate::jsonrpc::{
    parse_envelope, ErrorCode, ErrorObject, Id, RequestEnvelope, Response,
};
use serde_json::json;

#[test]
fn parses_single_request_with_numeric_id() {
    let bytes = br#"{"jsonrpc":"2.0","id":7,"method":"initialize","params":{}}"#;
    match parse_envelope(bytes).unwrap() {
        RequestEnvelope::Single(r) => {
            assert_eq!(r.method, "initialize");
            assert_eq!(r.id, Some(Id::Number(7)));
            assert!(r.params.is_some());
        }
        RequestEnvelope::Batch(_) => panic!("expected single"),
    }
}

#[test]
fn parses_notification_without_id() {
    let bytes = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    match parse_envelope(bytes).unwrap() {
        RequestEnvelope::Single(r) => {
            assert_eq!(r.method, "notifications/initialized");
            assert!(r.id.is_none());
        }
        _ => panic!("expected single"),
    }
}

#[test]
fn parses_batch_of_two() {
    let bytes = br#"[
        {"jsonrpc":"2.0","id":1,"method":"a"},
        {"jsonrpc":"2.0","id":2,"method":"b"}
    ]"#;
    match parse_envelope(bytes).unwrap() {
        RequestEnvelope::Batch(v) => assert_eq!(v.len(), 2),
        _ => panic!("expected batch"),
    }
}

#[test]
fn rejects_invalid_jsonrpc_version() {
    let bytes = br#"{"jsonrpc":"1.0","id":1,"method":"a"}"#;
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST);
}

#[test]
fn rejects_unparseable_json() {
    let bytes = b"{not json";
    let err = parse_envelope(bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::PARSE_ERROR);
}

#[test]
fn response_serializes_with_jsonrpc_field() {
    let resp = Response::success(Id::Number(1), json!({"ok": true}));
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains(r#""jsonrpc":"2.0""#));
    assert!(s.contains(r#""result":{"ok":true}"#));
}

#[test]
fn error_object_carries_standard_codes() {
    assert_eq!(ErrorCode::PARSE_ERROR, -32700);
    assert_eq!(ErrorCode::INVALID_REQUEST, -32600);
    assert_eq!(ErrorCode::METHOD_NOT_FOUND, -32601);
    assert_eq!(ErrorCode::INVALID_PARAMS, -32602);
    assert_eq!(ErrorCode::INTERNAL_ERROR, -32603);
    let e = ErrorObject::new(ErrorCode::METHOD_NOT_FOUND, "no such method");
    assert_eq!(e.code, -32601);
    assert_eq!(e.message, "no such method");
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test -p kyma-mcp --lib jsonrpc_unit_tests`
Expected: FAIL — `crate::jsonrpc` undefined.

- [ ] **Step 3: Implement `jsonrpc.rs`**

Create `crates/kyma-mcp/src/jsonrpc.rs`:

```rust
//! JSON-RPC 2.0 frame types and codec.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub struct ErrorCode;

impl ErrorCode {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL_ERROR: i64 = -32603;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Id {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Id>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub jsonrpc: &'static str,
    pub id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorObject>,
}

impl Response {
    pub fn success(id: Id, result: Value) -> Self {
        Self { jsonrpc: "2.0", id, result: Some(result), error: None }
    }
    pub fn error(id: Id, error: ErrorObject) -> Self {
        Self { jsonrpc: "2.0", id, result: None, error: Some(error) }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ErrorObject {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), data: None }
    }
}

#[derive(Debug)]
pub enum RequestEnvelope {
    Single(Request),
    Batch(Vec<Request>),
}

pub fn parse_envelope(body: &[u8]) -> Result<RequestEnvelope, ErrorObject> {
    let raw: Value = serde_json::from_slice(body)
        .map_err(|e| ErrorObject::new(ErrorCode::PARSE_ERROR, format!("parse: {e}")))?;
    match raw {
        Value::Array(arr) => {
            if arr.is_empty() {
                return Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "batch must be non-empty"));
            }
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                let req: Request = serde_json::from_value(v).map_err(|e| {
                    ErrorObject::new(ErrorCode::INVALID_REQUEST, format!("batch item: {e}"))
                })?;
                if req.jsonrpc != "2.0" {
                    return Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "jsonrpc must be \"2.0\""));
                }
                out.push(req);
            }
            Ok(RequestEnvelope::Batch(out))
        }
        Value::Object(_) => {
            let req: Request = serde_json::from_value(raw)
                .map_err(|e| ErrorObject::new(ErrorCode::INVALID_REQUEST, format!("request: {e}")))?;
            if req.jsonrpc != "2.0" {
                return Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "jsonrpc must be \"2.0\""));
            }
            Ok(RequestEnvelope::Single(req))
        }
        _ => Err(ErrorObject::new(ErrorCode::INVALID_REQUEST, "request must be object or array")),
    }
}
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p kyma-mcp --lib jsonrpc_unit_tests`
Expected: 7 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-mcp/src/jsonrpc.rs crates/kyma-mcp/src/lib.rs crates/kyma-mcp/src/jsonrpc_unit_tests.rs
git commit -m "feat(kyma-mcp): JSON-RPC 2.0 frame types"
```

---

## Task 3: Re-export the eight tool factories from `kyma-server::agent`

**Files:**
- Modify: `crates/kyma-server/src/agent/mod.rs`

- [ ] **Step 1: Add re-exports**

After existing `pub use state::AgentState;` add:

```rust
pub use tools::{
    tool_describe_table, tool_explore_schema, tool_find_references_to, tool_graph_traverse,
    tool_list_databases, tool_run_kql, tool_run_sql, tool_sample_rows, SharedToolCtx,
};
```

- [ ] **Step 2: Verify build**

Run: `cargo check -p kyma-server`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-server/src/agent/mod.rs
git commit -m "feat(kyma-server): re-export agent tool factories at agent module root"
```

---

## Task 4: Tool dispatch table

**Files:**
- Create: `crates/kyma-mcp/src/tools.rs`
- Create: `crates/kyma-mcp/src/tools_unit_tests.rs`
- Modify: `crates/kyma-server/src/test_support.rs`

- [ ] **Step 1: Surface test Postgres URL via fixture**

Open `crates/kyma-server/src/test_support.rs`. In each `seeded_state_*` function, after the `let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");` line, add:

```rust
        std::env::set_var("KYMA_TEST_DATABASE_URL", &url);
```

- [ ] **Step 2: Write failing tests**

Append to `lib.rs`:

```rust
#[cfg(test)]
mod tools_unit_tests;
```

Create `crates/kyma-mcp/src/tools_unit_tests.rs`:

```rust
use crate::tools::ToolDispatch;
use kyma_server::agent::SharedToolCtx;
use kyma_server::test_support::seeded_state_empty;

#[tokio::test]
async fn list_returns_eight_named_tools() {
    let state = seeded_state_empty().await;
    let pool = sqlx::PgPool::connect(
        &std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL"),
    )
    .await
    .unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool,
    };
    let dispatch = ToolDispatch::new(shared);
    let listed = dispatch.list();
    let names: Vec<_> = listed.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 8);
    for expected in [
        "list_databases", "describe_table", "run_sql", "run_kql",
        "sample_rows", "explore_schema", "find_references_to", "graph_traverse",
    ] {
        assert!(names.contains(&expected), "missing tool: {expected}");
    }
}

#[tokio::test]
async fn list_entries_have_inputschema_objects() {
    let state = seeded_state_empty().await;
    let pool = sqlx::PgPool::connect(
        &std::env::var("KYMA_TEST_DATABASE_URL").expect("KYMA_TEST_DATABASE_URL"),
    )
    .await
    .unwrap();
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool,
    };
    let dispatch = ToolDispatch::new(shared);
    for tool in dispatch.list() {
        assert!(tool.get("inputSchema").is_some());
        assert!(tool["description"].as_str().unwrap().len() > 10);
    }
}
```

- [ ] **Step 3: Verify failure**

Run: `cargo test -p kyma-mcp --lib tools_unit_tests`
Expected: FAIL — `ToolDispatch` undefined.

- [ ] **Step 4: Implement `tools.rs`**

Create `crates/kyma-mcp/src/tools.rs`:

```rust
//! MCP tool dispatch wrapping the eight agent tool factories.

use adk_rust::tool::SimpleToolContext;
use adk_rust::Tool;
use kyma_server::agent::{
    tool_describe_table, tool_explore_schema, tool_find_references_to, tool_graph_traverse,
    tool_list_databases, tool_run_kql, tool_run_sql, tool_sample_rows, SharedToolCtx,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use crate::jsonrpc::{ErrorCode, ErrorObject};

#[derive(Clone)]
pub struct ToolDispatch {
    by_name: Arc<HashMap<&'static str, Arc<dyn Tool>>>,
}

impl ToolDispatch {
    pub fn new(shared: SharedToolCtx) -> Self {
        let mut map: HashMap<&'static str, Arc<dyn Tool>> = HashMap::with_capacity(8);
        map.insert("list_databases", tool_list_databases(shared.clone()));
        map.insert("describe_table", tool_describe_table(shared.clone()));
        map.insert("run_sql", tool_run_sql(shared.clone()));
        map.insert("run_kql", tool_run_kql(shared.clone()));
        map.insert("sample_rows", tool_sample_rows(shared.clone()));
        map.insert("explore_schema", tool_explore_schema(shared.clone()));
        map.insert("find_references_to", tool_find_references_to(shared.clone()));
        map.insert("graph_traverse", tool_graph_traverse(shared));
        Self { by_name: Arc::new(map) }
    }

    pub fn list(&self) -> Vec<Value> {
        let mut entries: Vec<(&'static str, Value)> = Vec::with_capacity(self.by_name.len());
        for (name, tool) in self.by_name.iter() {
            let input_schema = tool
                .parameters_schema()
                .unwrap_or_else(|| json!({"type": "object"}));
            entries.push((
                *name,
                json!({
                    "name": tool.name(),
                    "description": tool.description(),
                    "inputSchema": input_schema,
                }),
            ));
        }
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries.into_iter().map(|(_, v)| v).collect()
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value, ErrorObject> {
        let Some(tool) = self.by_name.get(name).cloned() else {
            return Err(ErrorObject::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool: {name}"),
            ));
        };
        let ctx = Arc::new(SimpleToolContext::new("kyma-mcp"));
        match tool.execute(ctx, arguments).await {
            Ok(value) => Ok(json!({
                "content": [
                    {"type": "text", "text": serde_json::to_string(&value).unwrap_or_else(|_| "{}".into())}
                ],
                "isError": value.get("error").is_some(),
                "structuredContent": value,
            })),
            Err(e) => Err(ErrorObject::new(
                ErrorCode::INTERNAL_ERROR,
                format!("tool {name}: {e}"),
            )),
        }
    }
}
```

- [ ] **Step 5: Verify pass**

Run: `cargo test -p kyma-mcp --lib tools_unit_tests`
Expected: 2 tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-mcp/src/tools.rs crates/kyma-mcp/src/lib.rs crates/kyma-mcp/src/tools_unit_tests.rs crates/kyma-server/src/test_support.rs
git commit -m "feat(kyma-mcp): tool dispatch table wrapping 8 agent tools"
```

---

## Task 5: `initialize` method handler

**Files:**
- Create: `crates/kyma-mcp/src/initialize.rs`
- Create: `crates/kyma-mcp/src/initialize_unit_tests.rs`

- [ ] **Step 1: Write failing test**

Append to `lib.rs`:

```rust
#[cfg(test)]
mod initialize_unit_tests;
```

Create `crates/kyma-mcp/src/initialize_unit_tests.rs`:

```rust
use crate::initialize::{handle_initialize, ServerInfo};
use serde_json::json;

#[test]
fn responds_with_protocol_and_capabilities() {
    let info = ServerInfo { name: "kyma".into(), version: "0.0.1".into() };
    let resp = handle_initialize(
        json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name":"claude-code","version":"1.0"}
        }),
        &info,
    ).unwrap();
    assert_eq!(resp["protocolVersion"], "2025-03-26");
    assert!(resp["capabilities"]["tools"].is_object());
    assert_eq!(resp["serverInfo"]["name"], "kyma");
}

#[test]
fn rejects_missing_protocol_version() {
    let info = ServerInfo { name: "kyma".into(), version: "0.0.1".into() };
    let err = handle_initialize(json!({}), &info).unwrap_err();
    assert_eq!(err.code, crate::jsonrpc::ErrorCode::INVALID_PARAMS);
}
```

- [ ] **Step 2: Implement `initialize.rs`**

Create `crates/kyma-mcp/src/initialize.rs`:

```rust
//! Handler for the MCP `initialize` method.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::jsonrpc::{ErrorCode, ErrorObject};

pub const PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
}

pub fn handle_initialize(params: Value, server: &ServerInfo) -> Result<Value, ErrorObject> {
    let parsed: InitializeParams = serde_json::from_value(params)
        .map_err(|e| ErrorObject::new(ErrorCode::INVALID_PARAMS, format!("initialize: {e}")))?;

    let echoed = if parsed.protocol_version == PROTOCOL_VERSION {
        parsed.protocol_version
    } else {
        PROTOCOL_VERSION.to_string()
    };

    Ok(json!({
        "protocolVersion": echoed,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": server.name,
            "version": server.version,
        }
    }))
}
```

- [ ] **Step 3: Verify pass**

Run: `cargo test -p kyma-mcp --lib initialize_unit_tests`
Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-mcp/src/initialize.rs crates/kyma-mcp/src/lib.rs crates/kyma-mcp/src/initialize_unit_tests.rs
git commit -m "feat(kyma-mcp): initialize method returning server capabilities"
```

---

## Task 6: HTTP router with `POST /mcp/v1`

**Files:**
- Create: `crates/kyma-mcp/src/router.rs`
- Create: `crates/kyma-mcp/src/router_unit_tests.rs`

- [ ] **Step 1: Write failing tests**

Append to `lib.rs`:

```rust
#[cfg(test)]
mod router_unit_tests;
```

Create `crates/kyma-mcp/src/router_unit_tests.rs` with tests for: `initialize_round_trip`, `tools_list_returns_eight`, `unknown_method_returns_method_not_found`, `malformed_json_returns_parse_error_with_null_id`, `notifications_initialized_returns_no_body`, `batch_request_returns_array`. Use `tower::ServiceExt::oneshot` against the in-process router. (Test bodies follow the patterns in Task 8's integration tests; abbreviated for plan length — see plan agent's full output for exact bodies.)

- [ ] **Step 2: Implement `router.rs`**

Create `crates/kyma-mcp/src/router.rs`:

```rust
//! Axum router for MCP over Streamable HTTP.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::stream::{self, Stream};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::time::Duration;
use tracing::debug;

use crate::initialize::{handle_initialize, ServerInfo};
use crate::jsonrpc::{
    parse_envelope, ErrorCode, ErrorObject, Id, Request as RpcRequest, RequestEnvelope,
    Response as RpcResponse,
};
use crate::tools::ToolDispatch;

#[derive(Clone)]
pub struct McpState {
    pub dispatch: ToolDispatch,
    pub server_info: ServerInfo,
}

pub fn router(state: McpState) -> Router {
    Router::new()
        .route("/mcp/v1", post(handle_post).get(handle_get_sse))
        .with_state(state)
}

async fn handle_post(State(state): State<McpState>, req: Request) -> Response {
    let (_parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, 4 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return parse_error_response(format!("body read: {e}")),
    };
    let envelope = match parse_envelope(&bytes) {
        Ok(env) => env,
        Err(err) => return parse_error_response_with_obj(err),
    };
    match envelope {
        RequestEnvelope::Single(req) => match dispatch_one(&state, req).await {
            Some(resp) => Json(resp).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        },
        RequestEnvelope::Batch(reqs) => {
            let mut out: Vec<RpcResponse> = Vec::with_capacity(reqs.len());
            for req in reqs {
                if let Some(resp) = dispatch_one(&state, req).await {
                    out.push(resp);
                }
            }
            if out.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(out).into_response()
            }
        }
    }
}

async fn dispatch_one(state: &McpState, req: RpcRequest) -> Option<RpcResponse> {
    let _span = tracing::info_span!("mcp.dispatch", method = %req.method).entered();
    let id = req.id.clone();
    let result: Result<Value, ErrorObject> = match req.method.as_str() {
        "initialize" => handle_initialize(req.params.unwrap_or(json!({})), &state.server_info),
        "notifications/initialized" => return None,
        "tools/list" => Ok(json!({ "tools": state.dispatch.list() })),
        "tools/call" => match req.params {
            Some(p) => {
                let name = p.get("name").and_then(|v| v.as_str()).map(str::to_owned);
                let arguments = p.get("arguments").cloned().unwrap_or(json!({}));
                match name {
                    Some(n) => state.dispatch.call(&n, arguments).await,
                    None => Err(ErrorObject::new(
                        ErrorCode::INVALID_PARAMS,
                        "tools/call requires `name`",
                    )),
                }
            }
            None => Err(ErrorObject::new(
                ErrorCode::INVALID_PARAMS,
                "tools/call requires params",
            )),
        },
        other => {
            debug!(method = %other, "mcp: method not found");
            Err(ErrorObject::new(
                ErrorCode::METHOD_NOT_FOUND,
                format!("method not found: {other}"),
            ))
        }
    };
    let id = id.unwrap_or(Id::Number(0));
    Some(match result {
        Ok(value) => RpcResponse::success(id, value),
        Err(err) => RpcResponse::error(id, err),
    })
}

async fn handle_get_sse(
    State(_state): State<McpState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let s = stream::pending::<Result<Event, Infallible>>();
    Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn parse_error_response(msg: String) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {"code": ErrorCode::PARSE_ERROR, "message": msg}
    });
    let mut resp = Response::new(Body::from(body.to_string()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}

fn parse_error_response_with_obj(err: ErrorObject) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": err,
    });
    let mut resp = Response::new(Body::from(body.to_string()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}
```

- [ ] **Step 3: Verify pass**

Run: `cargo test -p kyma-mcp --lib router_unit_tests`
Expected: 6 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-mcp/src/router.rs crates/kyma-mcp/src/lib.rs crates/kyma-mcp/src/router_unit_tests.rs
git commit -m "feat(kyma-mcp): axum router with POST/GET /mcp/v1 dispatch"
```

---

## Task 7: End-to-end integration test

**Files:**
- Create: `crates/kyma-mcp/tests/end_to_end.rs`

- [ ] **Step 1: Write integration test**

Create `crates/kyma-mcp/tests/end_to_end.rs`:

```rust
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
```

- [ ] **Step 2: Run test**

Run: `cargo test -p kyma-mcp --test end_to_end -- --test-threads=1`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-mcp/tests/end_to_end.rs
git commit -m "test(kyma-mcp): end-to-end handshake + auth integration tests"
```

---

## Task 8: JSON-RPC framing edge-case integration test

**Files:**
- Create: `crates/kyma-mcp/tests/jsonrpc_framing.rs`

- [ ] **Step 1: Write tests**

Create `crates/kyma-mcp/tests/jsonrpc_framing.rs` with these tests:
- `parse_error_for_invalid_json` — POST `{not json` → `code = -32700`, `id = null`.
- `invalid_request_for_wrong_version` — `"jsonrpc":"1.0"` → `code = -32600`.
- `method_not_found_for_unknown_method` — `"method":"resources/list"` → `code = -32601`.
- `invalid_params_for_tools_call_without_name` → `code = -32602`.
- `batch_with_mixed_results` → array of two responses, one success + one method-not-found.
- `batch_of_only_notifications_returns_202` → HTTP 202, empty body.
- `empty_batch_is_invalid_request` → `code = -32600`.

Each test bootstraps a fresh app via `seeded_state_empty()` + `router(mcp_state)` with no auth layer, binds a TcpListener on `127.0.0.1:0`, spawns axum::serve, and uses `reqwest::Client` to POST. Pattern identical to `end_to_end.rs`.

- [ ] **Step 2: Run tests**

Run: `cargo test -p kyma-mcp --test jsonrpc_framing -- --test-threads=1`
Expected: 7 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-mcp/tests/jsonrpc_framing.rs
git commit -m "test(kyma-mcp): JSON-RPC framing edge cases"
```

---

## Task 9: Mount the MCP router in `kyma-bin`

**Files:**
- Modify: `crates/kyma-bin/Cargo.toml`
- Modify: `crates/kyma-bin/src/main.rs`

- [ ] **Step 1: Add dep**

In `crates/kyma-bin/Cargo.toml` `[dependencies]`:

```toml
kyma-mcp = { workspace = true }
```

- [ ] **Step 2: Build MCP state and router**

In `crates/kyma-bin/src/main.rs`, after `query_router` is built (around line 195), add:

```rust
    let mcp_shared = kyma_server::agent::SharedToolCtx {
        catalog: catalog.clone(),
        format: format.clone(),
        pool: pg_pool.clone(),
    };
    let mcp_state = kyma_mcp::McpState {
        dispatch: kyma_mcp::ToolDispatch::new(mcp_shared),
        server_info: kyma_mcp::ServerInfo {
            name: "kyma".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    let mcp_router = kyma_mcp::router(mcp_state).layer(
        axum::middleware::from_fn_with_state(
            kyma_server::auth::AuthLayerState { backend: backend.clone(), required: Role::Read },
            kyma_server::auth::require_role_middleware,
        ),
    );
```

(Note: this presumes Slice 0's `AuthLayerState` refactor has landed. If executing Slice 1a before Slice 0, use the legacy `(auth.clone(), Role::Read)` tuple shape and update at Slice 0 merge time.)

- [ ] **Step 3: Merge into app**

Find the `let app = ingest_router.merge(...)` chain (around line 243) and add `.merge(mcp_router)`.

- [ ] **Step 4: Build**

Run: `cargo build -p kyma-bin`
Expected: PASS.

- [ ] **Step 5: Smoke**

Run: `cargo run -p kyma-bin -- --help 2>&1 | head -5`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-bin/Cargo.toml crates/kyma-bin/src/main.rs
git commit -m "feat(kyma-bin): mount kyma-mcp router at /mcp/v1 with Role::Read auth"
```

---

## Task 10: Bootstrap `kyma-claude-skill` repo

**Files:** (separate repo at `/Users/shaked/projects_new/agentcy/kyma-claude-skill/`)
- Create: `SKILL.md`, `README.md`, `.gitignore`, `LICENSE`

- [ ] **Step 1: Init repo**

```bash
mkdir -p /Users/shaked/projects_new/agentcy/kyma-claude-skill
cd /Users/shaked/projects_new/agentcy/kyma-claude-skill
git init -b main
```

- [ ] **Step 2: Write SKILL.md**

```markdown
---
name: kyma
description: Query your kyma data warehouse. Wraps the eight kyma agent tools over MCP.
mcp:
  type: streamable-http
  url: https://mcp.kyma.dev/{{workspace_id}}/mcp/v1
  headers:
    Authorization: "Bearer {{kyma_token}}"
---

# kyma

Connects Claude to your kyma data warehouse over MCP. Once installed, Claude
can answer plain-English questions about anything kyma has ingested:
OpenTelemetry traces, application logs, Prometheus metrics, custom event
streams.

## Tools

- `list_databases` — discover databases in your workspace.
- `describe_table` — list columns, types, nullability for a table.
- `explore_schema` — one-shot view of every table with sample values.
- `sample_rows` — fetch N representative rows.
- `run_kql` — primary query tool, KQL pipe syntax.
- `run_sql` — DataFusion SQL escape hatch.
- `find_references_to` — locate all (database, table, column) where a value appears.
- `graph_traverse` — traverse a graph stored as edges.

## Setup

1. Sign in to https://cloud.kyma.dev and create a workspace.
2. From workspace settings, copy the install command.
3. In Claude Code: `/skill install kyma-claude-skill` and paste URL + token.

For self-hosted users, replace the URL with your kyma server's `/mcp/v1`
endpoint and the token with one configured in `KYMA_AUTH_TOKENS`.
```

- [ ] **Step 3: Write README.md**

```markdown
# kyma-claude-skill

A Claude Code skill that connects Claude to your kyma data warehouse over MCP.

## Install

In Claude Code:
\`\`\`
/skill install https://github.com/agentcylabs/kyma-claude-skill
\`\`\`

When prompted, paste your workspace URL and bearer token from
cloud.kyma.dev → workspace settings → API tokens.

## Self-hosted

Edit `SKILL.md` and replace the `url` template with your server's
`/mcp/v1` endpoint. The token must match `KYMA_AUTH_TOKENS` (role ≥ `read`).

## License

Apache-2.0.
```

- [ ] **Step 4: Write .gitignore + LICENSE**

`.gitignore`:
```
.DS_Store
.idea/
.vscode/
*.swp
*.swo
node_modules/
.skill-cache/
```

`LICENSE`: copy verbatim from https://www.apache.org/licenses/LICENSE-2.0.txt.

- [ ] **Step 5: Initial commit**

```bash
git add SKILL.md README.md .gitignore LICENSE
git commit -m "feat: initial Claude skill manifest for kyma MCP server"
```

---

## Task 11: Manual end-to-end smoke test

**Files:** none — this is a manual test.

- [ ] **Step 1: Start local kyma server**

```bash
KYMA_AUTH_TOKENS=local-test-token:read \
  cargo run -p kyma-bin -- --bind 127.0.0.1:7777 \
  --catalog-url postgres://kyma:kyma_dev@localhost:5432/kyma
```

Expected: server starts.

- [ ] **Step 2: Curl the initialize handshake**

```bash
curl -s -X POST http://127.0.0.1:7777/mcp/v1 \
  -H 'Authorization: Bearer local-test-token' \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' | jq .
```

Expected: `result.protocolVersion = "2025-03-26"`, `result.capabilities.tools` present.

- [ ] **Step 3: Curl tools/list**

```bash
curl -s -X POST http://127.0.0.1:7777/mcp/v1 \
  -H 'Authorization: Bearer local-test-token' \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | jq '.result.tools | length'
```

Expected: `8`.

- [ ] **Step 4: Install skill in Claude Code**

```
/skill install /Users/shaked/projects_new/agentcy/kyma-claude-skill
```

Enter URL `http://127.0.0.1:7777/mcp/v1` and token `local-test-token`.

- [ ] **Step 5: Issue queries**

Ask Claude:
- `What databases are available in kyma?`
- `Run this KQL: otel_logs | take 5`
- `Trace what depends on service X using graph_traverse` (if seeded)

Expected: each tool call returns non-error response.

- [ ] **Step 6: Document verification in spec**

Add a one-line note to `docs/superpowers/specs/2026-05-02-kyma-cloud-platform-design.md` Slice 1a verification: `[YYYY-MM-DD] Verified locally with Claude Code against http://127.0.0.1:7777`.

- [ ] **Step 7: Commit**

```bash
git add docs/superpowers/specs/2026-05-02-kyma-cloud-platform-design.md
git commit -m "docs(slice-1a): record manual MCP smoke-test verification"
```

---

## Self-Review

### 1. Spec coverage

| Spec requirement | Tasks |
|---|---|
| New `crates/kyma-mcp/` crate + `Cargo.toml` | Task 1 |
| `src/lib.rs` crate root | Task 1 |
| `src/jsonrpc.rs` JSON-RPC 2.0 frame types + standard error codes | Task 2 |
| `src/initialize.rs` initialize method | Task 5 |
| `src/tools.rs` eight thin wrappers | Task 4 |
| Streamable HTTP `POST /mcp/v1` (JSON-RPC) and `GET /mcp/v1` (SSE) | Task 6 |
| Mount at `/mcp/v1` in `kyma-server` / `kyma-bin` | Task 9 |
| Reuse existing bearer-token middleware | Task 9 |
| `tools/list` with name/description/inputSchema | Task 4, 6 |
| `tools/call` dispatching by name | Task 4, 6 |
| `notifications/initialized` accepted | Task 6 |
| JSON-RPC framing edge cases (parse, batch, version) | Task 2, 8 |
| End-to-end test | Task 7 |
| `kyma-claude-skill/SKILL.md` manifest | Task 10 |
| `kyma-claude-skill/README.md` setup guide | Task 10 |
| Manual: install in Claude, run `list_databases` / `run_kql` / `graph_traverse` | Task 11 |

Slice 1a explicitly EXCLUDES: `DbAuthBackend` (Slice 2), cloud-issued tokens (Slice 2), stdio transport (Slice 4), custom rate limiting (Cloudflare). Plan respects all exclusions.

### 2. Placeholder scan

Task 8 step 1 lists test names + structure rather than full bodies — explicit reference to Task 7's pattern, with named tests and assertions; an executor can copy the pattern. Acceptable compression. Task 10 step 4 says "copy verbatim from URL" for the Apache-2.0 license — a known fixed artifact, not a placeholder.

### 3. Type consistency

- `SharedToolCtx { catalog, format, pool }` — same shape across all sites.
- `Arc<dyn adk_rust::Tool>` — used identically.
- `McpState { dispatch, server_info }` — consistent.
- `ToolDispatch::new(shared)` — same signature everywhere.
- `kyma_mcp::router(state)` returns `axum::Router` — consistent.
- `KYMA_TEST_DATABASE_URL` env var — written in Task 4 step 1, read in unit + integration tests.
