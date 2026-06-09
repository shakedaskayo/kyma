//! HTTP handlers for `POST /v1/agent/ask` (SSE), `GET /v1/agent/runs/:run_id`,
//! and the engine-management surface (`GET/PUT /v1/agent/engine`, etc.).
//!
//! The ask handler drives an ADK-Rust `Runner` and emits a stream of SSE
//! frames. Once the stream completes (naturally, on error, or on budget
//! overrun) we insert one row into `agent_runs` with the full trace as JSONB.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use adk_rust::futures::StreamExt;
use adk_rust::identity::{SessionId, UserId};
use adk_rust::{Content, Part};
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::types::Json as SqlxJson;
use sqlx::PgPool;
use tracing::{debug, error, info, warn};

use super::engine::{
    build_engine, claude_cli, engine_catalogue, CredentialResolver, EngineConfig, EngineKind,
};
use super::memory_retrieve::{retrieve, RetrieveRequest};
use super::runner::{make_runner, model_id, run_oneshot, ANON_USER};
use super::sessions;
use super::state::AgentState;
use super::tools::{execute_sql, SharedToolCtx};
use super::ui_stream;
use crate::auth::{Principal, Role};

/// Hard cap on tool-call count per run. Above this, the turn is aborted
/// with `run_error{code="tool_loop"}` and persisted as `budget_exceeded`.
const MAX_TOOL_CALLS: u32 = 12;

/// Wall-clock deadline for the whole SSE exchange.
const RUN_WALL_CLOCK: Duration = Duration::from_secs(60);

/// Default number of new turns after which a session's rolling summary is
/// refreshed. Overridable via `KYMA_SESSION_SUMMARY_EVERY`.
const DEFAULT_SUMMARY_EVERY: i32 = 12;

fn summary_every() -> i32 {
    std::env::var("KYMA_SESSION_SUMMARY_EVERY")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_SUMMARY_EVERY)
}

pub fn router(state: AgentState) -> axum::Router {
    axum::Router::new()
        .route("/ask", post(ask_handler))
        .route("/runs/:run_id", get(run_lookup_handler))
        .route("/sessions", get(list_sessions_handler))
        .route(
            "/sessions/:session_id",
            get(get_session_handler).delete(delete_session_handler),
        )
        .route(
            "/sessions/:session_id/turns",
            get(get_session_turns_handler),
        )
        .route("/engines", get(list_engines))
        .route("/engine", get(get_engine).put(put_engine))
        .route("/engine/test", post(test_engine))
        .route("/skills", get(list_skills))
        .route(
            "/skills/enabled",
            get(get_enabled_skills).put(put_enabled_skills),
        )
        .route("/memory/overview", get(super::memory::overview_handler))
        .route("/memory/query", post(memory_query_handler))
        .route(
            "/memory/settings",
            get(get_memory_settings).put(put_memory_settings),
        )
        .route(
            "/retention/settings",
            get(get_retention_settings).put(put_retention_settings),
        )
        .route("/files/contribute", post(contribute_file_route))
        .route("/memory/export", get(export_memory_handler))
        .route("/memory/changes", get(changes_memory_handler))
        .route("/memory/import", post(import_memory_handler))
        .route(
            "/memory/dreaming/runs",
            get(super::dreaming::list_runs_handler),
        )
        .route(
            "/memory/dreaming/runs/:id",
            get(super::dreaming::get_run_handler),
        )
        .route(
            "/memory/dreaming/run",
            post(super::dreaming::trigger_run_handler),
        )
        .with_state(state)
}

#[derive(Debug, Deserialize, Default)]
struct ImportBody {
    #[serde(default)]
    memory_nodes: Vec<Value>,
    #[serde(default)]
    memory_edges: Vec<Value>,
}

/// `POST /v1/agent/memory/import` — apply memory node/edge rows (from another
/// instance's `export`/`changes`) into this store **via the MemoryWriter**, so
/// the canonical memory schema (typed `importance`, the embedding vector, …) and
/// on-demand provisioning are used — unlike generic `/v1/ingest`, which would
/// infer every column as text. Append-only / latest-wins, so re-applying is safe.
/// This is the receive side of memory sync.
async fn import_memory_handler(
    State(state): State<AgentState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<ImportBody>,
) -> Response {
    // Import is a bulk WRITE of memory nodes/edges from another instance. The
    // surrounding agent router is mounted at `Role::Read`, so gate this one
    // route at `Role::Write` in-handler — a read-only token must not be able to
    // push memory into the store.
    if principal.role < Role::Write {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "memory import requires write role" })),
        )
            .into_response();
    }
    let embed = match kyma_memory::shared_embedding().await {
        Ok(e) => e,
        Err(e) => {
            return Json(json!({ "error": format!("embedding backend: {e}") })).into_response()
        }
    };
    let writer = kyma_memory::MemoryWriter::new(state.catalog.clone(), state.format.clone(), embed);
    let mut applied_nodes = 0usize;
    let mut applied_edges = 0usize;
    let mut errors: Vec<String> = Vec::new();
    if !body.memory_nodes.is_empty() {
        match writer.append_node_rows(body.memory_nodes.clone()).await {
            Ok(()) => applied_nodes = body.memory_nodes.len(),
            Err(e) => errors.push(format!("nodes: {e}")),
        }
    }
    if !body.memory_edges.is_empty() {
        match writer.append_edge_rows(body.memory_edges.clone()).await {
            Ok(()) => applied_edges = body.memory_edges.len(),
            Err(e) => errors.push(format!("edges: {e}")),
        }
    }
    Json(json!({
        "applied_nodes": applied_nodes,
        "applied_edges": applied_edges,
        "errors": errors,
    }))
    .into_response()
}

#[derive(Debug, Deserialize)]
struct ExportParams {
    /// Restrict the export to one realm. Omit to export all.
    #[serde(default)]
    realm: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChangesParams {
    /// Return only memories changed strictly after this RFC3339 timestamp.
    /// Omit for the full set (epoch). Pass back the response's `until` next time.
    #[serde(default)]
    since: Option<String>,
    /// Restrict to one realm. Omit for all.
    #[serde(default)]
    realm: Option<String>,
}

/// `GET /v1/agent/memory/changes?since=<rfc3339>[&realm=]` — the incremental
/// pull side of memory sync: latest node versions whose `updated_at` (and edges
/// whose `created_at`) are strictly after `since`, plus an `until` watermark
/// (server clock) the caller advances to for the next pull. Same row shape as
/// export, so the caller re-applies via `POST /v1/ingest` (X-Database: memory).
async fn changes_memory_handler(
    State(state): State<AgentState>,
    axum::extract::Query(params): axum::extract::Query<ChangesParams>,
) -> Json<Value> {
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
        memory: state.memory.clone(),
    };
    let since = params
        .since
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
    let since_esc = since.replace('\'', "''");
    let realm_filter = params
        .realm
        .as_deref()
        .map(|r| format!(" AND realm = '{}'", r.replace('\'', "''")))
        .unwrap_or_default();
    let nodes_sql = format!(
        "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM memory_nodes) \
         SELECT id, labels, realm, memory_type, title, content, content_preview, tags, importance, status, \
                source_session_id, source_run_id, embedding, created_at, updated_at, \
                valid_at, invalid_at, superseded_by, provenance, topic_key \
         FROM latest WHERE __rn = 1 AND updated_at > '{since_esc}'{realm_filter}"
    );
    let edges_sql = format!(
        "SELECT id, src, dst, type, realm, target_namespace, props, created_at \
         FROM memory_edges WHERE created_at > '{since_esc}'"
    );
    let db = kyma_memory::DEFAULT_DATABASE;
    let nodes = execute_sql(&shared, db, &nodes_sql, 1_000_000).await;
    let edges = execute_sql(&shared, db, &edges_sql, 1_000_000).await;
    let rows = |v: Value| v.get("rows").cloned().unwrap_or_else(|| json!([]));
    Json(json!({
        "since": since,
        "until": Utc::now().to_rfc3339(),
        "memory_nodes": rows(nodes),
        "memory_edges": rows(edges),
    }))
}

/// `GET /v1/agent/memory/export[?realm=]` — full memory snapshot (latest node
/// versions + edges, including embeddings) as JSON, for backup / portability.
/// Re-import on another instance via the idempotent `POST /v1/ingest`
/// (`X-Database: memory`, `X-Table: memory_nodes|memory_edges`, NDJSON).
async fn export_memory_handler(
    State(state): State<AgentState>,
    axum::extract::Query(params): axum::extract::Query<ExportParams>,
) -> Json<Value> {
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
        memory: state.memory.clone(),
    };
    let realm_filter = params
        .realm
        .as_deref()
        .map(|r| format!(" AND realm = '{}'", r.replace('\'', "''")))
        .unwrap_or_default();
    let nodes_sql = format!(
        "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn FROM memory_nodes) \
         SELECT id, labels, realm, memory_type, title, content, content_preview, tags, importance, status, \
                source_session_id, source_run_id, embedding, created_at, updated_at, \
                valid_at, invalid_at, superseded_by, provenance, topic_key \
         FROM latest WHERE __rn = 1{realm_filter}"
    );
    let edges_sql =
        "SELECT id, src, dst, type, realm, target_namespace, props, created_at FROM memory_edges";
    let db = kyma_memory::DEFAULT_DATABASE;
    let nodes = execute_sql(&shared, db, &nodes_sql, 1_000_000).await;
    let edges = execute_sql(&shared, db, edges_sql, 1_000_000).await;
    let rows = |v: Value| v.get("rows").cloned().unwrap_or_else(|| json!([]));
    Json(json!({
        "memory_nodes": rows(nodes),
        "memory_edges": rows(edges),
        "hint": "Re-import on another instance via POST /v1/agent/memory/import \
                 with this same {memory_nodes, memory_edges} body (writes through \
                 the MemoryWriter, preserving the canonical schema + embeddings).",
    }))
}

/// `GET /v1/agent/memory/settings` — current tunable memory settings. Reads the
/// Postgres row in server mode, or the local JSON file in local mode.
async fn get_memory_settings(State(state): State<AgentState>) -> Json<Value> {
    let s = super::memory_settings::load_for(&state).await;
    Json(serde_json::to_value(s).unwrap_or_else(|_| json!({})))
}

/// `PUT /v1/agent/memory/settings` — persist tunable memory settings. Server
/// mode writes the Postgres row; local mode writes the JSON file under
/// `${KYMA_HOME}`.
async fn put_memory_settings(
    State(state): State<AgentState>,
    Json(body): Json<super::memory_settings::MemorySettings>,
) -> Response {
    if let Some(pool) = state.pool.as_ref() {
        return match super::memory_settings::save(pool, state.tenant, &body).await {
            Ok(()) => Json(json!({ "ok": true })).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        };
    }
    // Local mode: persist to the settings file so dreaming knobs survive restarts.
    if let Some(path) = state.memory_settings_path.as_ref() {
        return match super::memory_settings::save_local(path, &body).await {
            Ok(()) => Json(json!({ "ok": true, "persisted": true })).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response(),
        };
    }
    Json(json!({ "ok": true, "persisted": false })).into_response()
}

/// `POST /v1/agent/files/contribute` — persist a file's structure as candidate
/// graph nodes (used by `kyma scrape` / `kyma watch` and any HTTP client). Same
/// path as the `contribute_file` MCP tool.
#[derive(Debug, serde::Deserialize)]
struct ContributeFileBody {
    path: String,
    #[serde(default)]
    realm: String,
    repo: Option<String>,
    content: String,
    why_read: Option<String>,
}

async fn contribute_file_route(
    State(state): State<AgentState>,
    Json(body): Json<ContributeFileBody>,
) -> Response {
    match super::file_tools::contribute_file_impl(
        state.catalog.clone(),
        state.format.clone(),
        body.path,
        body.realm,
        body.repo,
        body.content,
        body.why_read,
    )
    .await
    {
        Ok(s) => Json(serde_json::to_value(s).unwrap_or_else(|_| json!({}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e })),
        )
            .into_response(),
    }
}

/// `GET /v1/agent/retention/settings` — current data-retention settings
/// (global / per-source / per-table / per-artifact-class day-counts).
async fn get_retention_settings(State(state): State<AgentState>) -> Response {
    let Some(pg) = state
        .catalog
        .as_ref_any()
        .downcast_ref::<kyma_catalog::PostgresCatalog>()
    else {
        // Local mode: no Postgres, settings default to retain-forever.
        return Json(kyma_core::retention::RetentionSettings::default()).into_response();
    };
    match pg.load_retention_settings(state.tenant).await {
        Ok(s) => Json(s).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// `PUT /v1/agent/retention/settings` — persist settings, then apply: stamp
/// `artifacts.expires_at` (so the retention worker sweeps them) and reconcile
/// explicit per-table overrides into `tables.config.retention_days`.
async fn put_retention_settings(
    State(state): State<AgentState>,
    Json(body): Json<kyma_core::retention::RetentionSettings>,
) -> Response {
    // A day-count of 0 would mean "expire immediately" — almost always a
    // foot-gun. Retain-forever is expressed by omitting the key (or null global).
    let has_zero = body.global_default_days == Some(0)
        || body.per_source_days.values().any(|&d| d == 0)
        || body.per_table_days.values().any(|&d| d == 0)
        || body.per_artifact_class_days.values().any(|&d| d == 0);
    if has_zero {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "retention days must be >= 1; omit a key (or null global_default_days) to retain forever",
            })),
        )
            .into_response();
    }

    let Some(pg) = state
        .catalog
        .as_ref_any()
        .downcast_ref::<kyma_catalog::PostgresCatalog>()
    else {
        return Json(json!({ "ok": true, "persisted": false })).into_response();
    };

    if let Err(e) = pg.save_retention_settings(state.tenant, &body).await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    // Apply immediately so the change is visible without waiting for a worker
    // tick. Both are idempotent; failures here are non-fatal (the worker
    // re-stamps artifacts on its next pass anyway).
    let stamped = pg
        .restamp_artifact_expiry(state.tenant, &body)
        .await
        .unwrap_or(0);
    if let Err(e) = pg.reconcile_table_retention(state.tenant, &body).await {
        tracing::warn!(error = %e, "reconcile_table_retention failed");
    }
    Json(json!({ "ok": true, "artifacts_restamped": stamped })).into_response()
}

/// `POST /v1/agent/memory/query` — near-realtime memory recall for external
/// callers (web UI, Claude Code hooks, coding agents). `mode: "fast"` (default)
/// runs the LLM-free graph-aware hybrid retrieval; `mode: "agentic"` adds a
/// short synthesized `brief` over the retrieved context.
#[derive(Debug, Deserialize)]
struct MemoryQueryRequest {
    #[serde(flatten)]
    retrieve: RetrieveRequest,
    #[serde(default)]
    mode: Option<String>,
}

async fn memory_query_handler(
    State(state): State<AgentState>,
    Json(body): Json<MemoryQueryRequest>,
) -> Json<Value> {
    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
        memory: state.memory.clone(),
    };
    let result = retrieve(&shared, &body.retrieve).await;
    let mut out = result.to_json();
    if body.mode.as_deref() == Some("agentic") && !result.context.is_empty() {
        let prompt = format!("Question: {}\n\n{}", body.retrieve.query, result.context);
        if let Ok(brief) = run_oneshot(
            &state,
            "kyma-memory-brief",
            "Answers a question from retrieved memory.",
            "Answer the question using ONLY the provided memories. Be concise and cite memory \
             ids. If the memories don't contain the answer, say so plainly.",
            &prompt,
        )
        .await
        {
            if let Value::Object(ref mut m) = out {
                m.insert("brief".into(), Value::String(brief));
            }
        }
    }
    Json(out)
}

#[derive(Debug, Deserialize)]
struct AskRequest {
    question: String,
    #[serde(default)]
    #[allow(dead_code)]
    database: Option<String>,
    #[serde(default)]
    include_thinking: bool,
    /// Optional conversation session. When present, prior turns are replayed
    /// for context and this turn is appended. A new id is minted when absent.
    #[serde(default)]
    session_id: Option<String>,
    /// Marks the session source (e.g. `"claude_code"` for hook-driven asks).
    #[serde(default)]
    source: Option<String>,
}

/// One recorded logical event — we stash both the event name and the JSON
/// body so we can persist the full trace into `agent_runs` after the stream
/// completes. (The persisted format is internal and independent of the wire
/// protocol the client sees.)
#[derive(Debug, Clone)]
struct TraceFrame {
    event: &'static str,
    data: Value,
}

/// Translates the agent's logical events into the AI-SDK **UI Message Stream**
/// the client consumes, while also recording a [`TraceFrame`] of each event for
/// persistence. One `Emitter` drives one assistant message: it opens the
/// message on construction and manages text/reasoning block lifecycles (so the
/// wire always has matching `*-start`/`*-end` parts) and tool-call/result
/// pairing by id.
struct Emitter {
    ui: ui_stream::UiStream,
    trace: Vec<TraceFrame>,
    /// Open text block id, if any.
    text_id: Option<String>,
    /// Open reasoning block id, if any.
    reasoning_id: Option<String>,
    /// Monotonic counter for unique block ids within the message.
    block_seq: u64,
    /// FIFO of synthetic tool-call ids per tool name, so a `tool_result`
    /// (which carries only the tool name) can be paired with its `tool_call`.
    tool_ids: HashMap<String, VecDeque<String>>,
    tool_seq: u64,
}

impl Emitter {
    /// Open the assistant message (`start` + `start-step`) and return the emitter.
    fn new(ui: ui_stream::UiStream, message_id: &str) -> Self {
        ui.start(message_id);
        ui.start_step();
        Self {
            ui,
            trace: Vec::new(),
            text_id: None,
            reasoning_id: None,
            block_seq: 0,
            tool_ids: HashMap::new(),
            tool_seq: 0,
        }
    }

    fn record(&mut self, event: &'static str, data: Value) {
        self.trace.push(TraceFrame { event, data });
    }

    fn next_block(&mut self) -> String {
        let id = format!("blk-{}", self.block_seq);
        self.block_seq += 1;
        id
    }

    fn close_text(&mut self) {
        if let Some(id) = self.text_id.take() {
            self.ui.text_end(&id);
        }
    }
    fn close_reasoning(&mut self) {
        if let Some(id) = self.reasoning_id.take() {
            self.ui.reasoning_end(&id);
        }
    }

    /// Surface the conversation session id so the client can resume.
    fn session(&mut self, session_id: &str) {
        self.record("session", json!({ "session_id": session_id }));
        self.ui.data("session", json!({ "sessionId": session_id }));
    }

    fn run_started(&mut self, run_id: &str, model: &str, question: &str) {
        self.record(
            "run_started",
            json!({ "run_id": run_id, "model": model, "question": question }),
        );
        self.ui.data("model", json!({ "model": model }));
    }

    fn answer_delta(&mut self, text: &str) {
        self.record("answer_delta", json!({ "text": text }));
        self.close_reasoning();
        let id = match &self.text_id {
            Some(id) => id.clone(),
            None => {
                let id = self.next_block();
                self.ui.text_start(&id);
                self.text_id = Some(id.clone());
                id
            }
        };
        self.ui.text_delta(&id, text);
    }

    fn thinking_delta(&mut self, text: &str) {
        self.record("thinking_delta", json!({ "text": text }));
        self.close_text();
        let id = match &self.reasoning_id {
            Some(id) => id.clone(),
            None => {
                let id = self.next_block();
                self.ui.reasoning_start(&id);
                self.reasoning_id = Some(id.clone());
                id
            }
        };
        self.ui.reasoning_delta(&id, text);
    }

    fn tool_call(&mut self, tool: &str, args: Value, call_index: u32) {
        self.record(
            "tool_call",
            json!({ "tool": tool, "args": args, "call_index": call_index }),
        );
        self.close_text();
        self.close_reasoning();
        let id = format!("call-{}", self.tool_seq);
        self.tool_seq += 1;
        self.tool_ids
            .entry(tool.to_string())
            .or_default()
            .push_back(id.clone());
        self.ui.tool_input_available(&id, tool, args);
    }

    fn tool_result(&mut self, tool: &str, result: Value) {
        self.record("tool_result", json!({ "tool": tool, "result": result }));
        let id = self
            .tool_ids
            .get_mut(tool)
            .and_then(|q| q.pop_front())
            .unwrap_or_else(|| format!("call-{tool}"));
        self.ui.tool_output_available(&id, result);
    }

    /// Final answer bookkeeping. The text itself has already streamed via
    /// `answer_delta`, so here we only close the block and surface the SQL/KQL
    /// the run used as data parts.
    fn answer_final(&mut self, text: &str, sql_used: Option<&str>, kql_used: Option<&str>) {
        self.record(
            "answer_final",
            json!({ "text": text, "kql_used": kql_used, "sql_used": sql_used }),
        );
        self.close_text();
        self.close_reasoning();
        if let Some(sql) = sql_used {
            self.ui.data("sql", json!({ "sql": sql }));
        }
        if let Some(kql) = kql_used {
            self.ui.data("kql", json!({ "kql": kql }));
        }
    }

    fn run_error(&mut self, code: &str, message: &str) {
        self.record("run_error", json!({ "code": code, "message": message }));
        self.ui.error(message);
    }

    /// Close the message: flush open blocks, emit run metadata, then the
    /// terminal `finish-step`/`finish`/`[DONE]` parts.
    fn finish(&mut self, usage: Value) {
        self.close_text();
        self.close_reasoning();
        self.ui.data("usage", usage);
        self.ui.finish_step();
        self.ui.finish();
        self.ui.done();
    }

    /// Build the JSON trace array persisted into `agent_runs`.
    fn trace_json(&self) -> Value {
        Value::Array(
            self.trace
                .iter()
                .map(|f| json!({ "event": f.event, "data": f.data }))
                .collect(),
        )
    }
}

/// POST /v1/agent/ask — run one agent turn and stream SSE frames.
async fn ask_handler(
    State(state): State<AgentState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<AskRequest>,
) -> Response {
    let question = body.question.trim().to_string();
    if question.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "question must be non-empty"})),
        )
            .into_response();
    }

    // Engine-kind branch: the Claude CLI engine owns its own tool loop, OAuth,
    // skills, and MCPs — adk-rust can't wrap it. Divert here before building
    // the runner.
    if let Ok(cfg) = state.engines.get().await {
        if cfg.kind == EngineKind::ClaudeCli {
            // `session_id` here is Claude's own session id (echoed back from a
            // prior turn's `data-session` part), used to `--resume`. The
            // caller's Authorization header is forwarded so Claude can reach
            // our own MCP endpoint (`mcp_url`) as the same principal.
            let auth_header = headers
                .get(axum::http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            return ask_via_claude_cli(
                state.pool.clone(),
                &question,
                &cfg,
                body.session_id.clone(),
                state.mcp_url.clone(),
                auth_header,
            )
            .await;
        }
    }

    let run_id = uuid::Uuid::new_v4();
    let include_thinking = body.include_thinking;
    let model = model_id(&state).await;
    let started_at = Utc::now();
    let start = Instant::now();

    // Resolve (or create) the conversation session (A5). Prior turns are
    // replayed into the runner; this turn is appended below.
    let source = body.source.as_deref().unwrap_or("kyma");
    let tenant_uuid = state.tenant.as_uuid();
    let sctx = sessions::load_or_create(
        state.pool.as_ref(),
        body.session_id.as_deref(),
        tenant_uuid,
        ANON_USER,
        source,
    )
    .await;
    let session_uuid = sctx.session_id;
    let session_id_str = session_uuid.to_string();
    let user_turn_index = sctx.next_turn_index;
    let assistant_turn_index = user_turn_index + 1;

    info!(run_id = %run_id, session_id = %session_uuid, question = %question, "agent run starting");

    // Record the user turn up-front so it survives even if the run errors.
    sessions::persist_turn(
        state.pool.as_ref(),
        session_uuid,
        tenant_uuid,
        user_turn_index,
        "user",
        &question,
        None,
    )
    .await;

    // Build runner up-front so we can surface init errors as a single-message
    // UI Message Stream (rather than an HTTP 500).
    let runner = match make_runner(
        &state,
        &session_id_str,
        &sctx.history,
        sctx.summary.as_deref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(run_id = %run_id, error = %e, "failed to build agent runner");
            let (ui, rx) = ui_stream::channel();
            let mut em = Emitter::new(ui, &run_id.to_string());
            em.session(&session_id_str);
            em.run_error("init_error", &e.to_string());
            em.finish(json!({ "tool_calls": 0, "elapsed_ms": start.elapsed().as_millis() as u64 }));
            // Best-effort persistence — ignore errors here since the
            // primary surface (SSE) already told the client.
            let _ = persist_run(
                state.pool.as_ref(),
                run_id,
                &question,
                &model,
                tenant_uuid,
                Some(session_uuid),
                started_at,
                Utc::now(),
                "error",
                &Value::Null,
                &em.trace_json(),
            )
            .await;
            return ui_stream::response(rx);
        }
    };

    // Spawn the driver task. The task owns the runner and the event loop and
    // drives the UI Message Stream through an `Emitter`; the response is a
    // `tokio_stream::UnboundedReceiverStream` over the parts it produces.
    let (ui, rx) = ui_stream::channel();
    let pool = state.pool.clone();
    let summary_state = state.clone();
    let summary_every = summary_every();

    tokio::spawn(async move {
        let mut em = Emitter::new(ui, &run_id.to_string());
        let mut tool_calls: u32 = 0;
        let mut last_run_sql: Option<String> = None;
        let mut final_text: String = String::new();

        // Surface the session id first so the client can capture it even if
        // the run errors mid-stream.
        em.session(&session_id_str);
        em.run_started(&run_id.to_string(), &model, &question);

        let content = Content::new("user").with_text(&question);

        let user_id = match UserId::new(ANON_USER) {
            Ok(u) => u,
            Err(e) => {
                em.run_error("internal", &format!("user_id: {e}"));
                finish_and_persist(
                    pool.as_ref(),
                    &mut em,
                    run_id,
                    &question,
                    &model,
                    tenant_uuid,
                    Some(session_uuid),
                    started_at,
                    start,
                    tool_calls,
                    "error",
                )
                .await;
                return;
            }
        };
        let session_id = match SessionId::new(&session_id_str) {
            Ok(s) => s,
            Err(e) => {
                em.run_error("internal", &format!("session_id: {e}"));
                finish_and_persist(
                    pool.as_ref(),
                    &mut em,
                    run_id,
                    &question,
                    &model,
                    tenant_uuid,
                    Some(session_uuid),
                    started_at,
                    start,
                    tool_calls,
                    "error",
                )
                .await;
                return;
            }
        };

        let run_future = async {
            let mut stream = match runner.run(user_id, session_id, content).await {
                Ok(s) => s,
                Err(e) => {
                    return Err(format!("runner.run: {e}"));
                }
            };

            while let Some(ev_result) = stream.next().await {
                let ev = match ev_result {
                    Ok(e) => e,
                    Err(e) => return Err(format!("event: {e}")),
                };

                let partial = ev.llm_response.partial;
                let parts_iter = ev
                    .llm_response
                    .content
                    .iter()
                    .flat_map(|c| c.parts.iter().cloned())
                    .collect::<Vec<Part>>();

                for part in parts_iter {
                    match part {
                        Part::Text { text } => {
                            // Non-partial Text means we have the complete
                            // (possibly aggregated) turn answer — stash it for
                            // the session record. Either way it streams as a
                            // text delta.
                            if !partial {
                                final_text.push_str(&text);
                            }
                            em.answer_delta(&text);
                        }
                        Part::Thinking { thinking, .. } => {
                            if include_thinking {
                                em.thinking_delta(&thinking);
                            }
                        }
                        Part::FunctionCall { name, args, .. } => {
                            tool_calls += 1;
                            if name == "run_sql" {
                                if let Some(s) = args.get("sql").and_then(|v| v.as_str()) {
                                    last_run_sql = Some(s.to_string());
                                }
                            }
                            em.tool_call(&name, args, tool_calls);
                            if tool_calls > MAX_TOOL_CALLS {
                                return Err(format!("tool_loop:{}", tool_calls));
                            }
                        }
                        Part::FunctionResponse {
                            function_response, ..
                        } => {
                            em.tool_result(&function_response.name, function_response.response);
                        }
                        _ => {}
                    }
                }

                if ev.is_final_response() {
                    // Fold in any final text even if it arrived as a
                    // non-partial full text part above. Nothing to do
                    // here — we've already pushed it to `final_text`.
                    debug!(run_id = %run_id, "agent emitted is_final_response");
                }
            }
            Ok::<(), String>(())
        };

        let outcome = tokio::time::timeout(RUN_WALL_CLOCK, run_future).await;

        let (status, status_str): (&str, &str) = match outcome {
            Ok(Ok(())) => ("success", "success"),
            Ok(Err(msg)) if msg.starts_with("tool_loop:") => {
                em.run_error("tool_loop", &msg);
                ("budget_exceeded", "budget_exceeded")
            }
            Ok(Err(msg)) => {
                em.run_error("runner_error", &msg);
                ("error", "error")
            }
            Err(_elapsed) => {
                warn!(run_id = %run_id, "agent run exceeded 60s wall clock");
                em.run_error(
                    "timeout",
                    &format!(
                        "agent run exceeded {}s wall clock budget",
                        RUN_WALL_CLOCK.as_secs()
                    ),
                );
                ("budget_exceeded", "budget_exceeded")
            }
        };

        if status == "success" {
            em.answer_final(&final_text, last_run_sql.as_deref(), None);
            // Record the assistant turn, then refresh the rolling summary.
            sessions::persist_turn(
                pool.as_ref(),
                session_uuid,
                tenant_uuid,
                assistant_turn_index,
                "assistant",
                &final_text,
                None,
            )
            .await;
            sessions::maybe_summarize_detached(summary_state, session_uuid, summary_every);
        }

        finish_and_persist(
            pool.as_ref(),
            &mut em,
            run_id,
            &question,
            &model,
            tenant_uuid,
            Some(session_uuid),
            started_at,
            start,
            tool_calls,
            status_str,
        )
        .await;
    });

    ui_stream::response(rx)
}

#[allow(clippy::too_many_arguments)]
async fn finish_and_persist(
    pool: Option<&PgPool>,
    em: &mut Emitter,
    run_id: uuid::Uuid,
    question: &str,
    model: &str,
    tenant: uuid::Uuid,
    session_id: Option<uuid::Uuid>,
    started_at: chrono::DateTime<Utc>,
    start: Instant,
    tool_calls: u32,
    status: &str,
) {
    let elapsed_ms = start.elapsed().as_millis() as u64;
    let usage_json = json!({
        "run_id": run_id.to_string(),
        "tool_calls": tool_calls,
        "elapsed_ms": elapsed_ms,
    });
    // Closes any open blocks and emits the terminal finish/[DONE] parts.
    em.finish(usage_json.clone());

    let trace_json = em.trace_json();

    if let Err(e) = persist_run(
        pool,
        run_id,
        question,
        model,
        tenant,
        session_id,
        started_at,
        Utc::now(),
        status,
        &usage_json,
        &trace_json,
    )
    .await
    {
        error!(run_id = %run_id, error = %e, "failed to persist agent_runs row");
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn persist_run(
    pool: Option<&PgPool>,
    run_id: uuid::Uuid,
    question: &str,
    model_id: &str,
    tenant: uuid::Uuid,
    session_id: Option<uuid::Uuid>,
    started_at: chrono::DateTime<Utc>,
    finished_at: chrono::DateTime<Utc>,
    status: &str,
    usage_json: &Value,
    trace_json: &Value,
) -> sqlx::Result<()> {
    let Some(pool) = pool else { return Ok(()) }; // local mode: no run persistence
    sqlx::query(
        r#"
        INSERT INTO agent_runs (
            run_id, tenant_id, question, model_id, auth_subject,
            session_id, started_at, finished_at, status,
            usage_json, trace_json, replay_cache_hit
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
    )
    .bind(run_id)
    .bind(tenant)
    .bind(question)
    .bind(model_id)
    .bind(ANON_USER)
    .bind(session_id)
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(SqlxJson(usage_json.clone()))
    .bind(SqlxJson(trace_json.clone()))
    .bind(false)
    .execute(pool)
    .await
    .map(|_| ())
}

// ---------------------------------------------------------------------------
// GET/DELETE /v1/agent/sessions* — multi-turn session surface (A5)
// ---------------------------------------------------------------------------

fn parse_session_id(raw: &str) -> Result<uuid::Uuid, Response> {
    uuid::Uuid::parse_str(raw).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid session_id (expected uuid)"})),
        )
            .into_response()
    })
}

async fn list_sessions_handler(State(state): State<AgentState>) -> Response {
    let Some(pool) = state.pool.as_ref() else {
        return Json(json!({ "sessions": [] })).into_response(); // local: no session history
    };
    let rows: Vec<(
        uuid::Uuid,
        Option<String>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        String,
        i64,
    )> = match sqlx::query_as(
        r#"
        SELECT s.session_id, s.title, s.created_at, s.last_active, s.source,
               COUNT(t.turn_index) AS turn_count
        FROM agent_sessions s
        LEFT JOIN agent_session_turns t ON t.session_id = s.session_id
        GROUP BY s.session_id
        ORDER BY s.last_active DESC
        LIMIT 200
        "#,
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "list sessions failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let sessions: Vec<Value> = rows
        .into_iter()
        .map(|(id, title, created, last_active, source, turn_count)| {
            json!({
                "session_id": id.to_string(),
                "title": title,
                "created_at": created,
                "last_active": last_active,
                "source": source,
                "turn_count": turn_count,
            })
        })
        .collect();
    Json(json!({ "sessions": sessions })).into_response()
}

async fn get_session_handler(
    State(state): State<AgentState>,
    Path(session_id): Path<String>,
) -> Response {
    let sid = match parse_session_id(&session_id) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let Some(pool) = state.pool.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response();
    };
    let row: Option<(
        Option<String>,
        Option<String>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        String,
        i32,
    )> = sqlx::query_as(
        "SELECT title, rolling_summary, created_at, last_active, source, summary_turn_index \
         FROM agent_sessions WHERE session_id = $1",
    )
    .bind(sid)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    match row {
        Some((title, summary, created, last_active, source, summary_idx)) => Json(json!({
            "session_id": sid.to_string(),
            "title": title,
            "rolling_summary": summary,
            "created_at": created,
            "last_active": last_active,
            "source": source,
            "summary_turn_index": summary_idx,
        }))
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response(),
    }
}

async fn get_session_turns_handler(
    State(state): State<AgentState>,
    Path(session_id): Path<String>,
) -> Response {
    let sid = match parse_session_id(&session_id) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let Some(pool) = state.pool.as_ref() else {
        return Json(json!({ "session_id": sid.to_string(), "turns": [] })).into_response();
    };
    let rows: Vec<(
        i32,
        String,
        SqlxJson<Value>,
        Option<uuid::Uuid>,
        chrono::DateTime<Utc>,
    )> = sqlx::query_as(
        "SELECT turn_index, role, content_json, run_id, created_at \
         FROM agent_session_turns WHERE session_id = $1 ORDER BY turn_index ASC",
    )
    .bind(sid)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let turns: Vec<Value> = rows
        .into_iter()
        .map(|(idx, role, content, run_id, created)| {
            json!({
                "turn_index": idx,
                "role": role,
                "content": content.0,
                "run_id": run_id.map(|r| r.to_string()),
                "created_at": created,
            })
        })
        .collect();
    Json(json!({ "session_id": sid.to_string(), "turns": turns })).into_response()
}

async fn delete_session_handler(
    State(state): State<AgentState>,
    Path(session_id): Path<String>,
) -> Response {
    let sid = match parse_session_id(&session_id) {
        Ok(u) => u,
        Err(resp) => return resp,
    };
    let Some(pool) = state.pool.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response();
    };
    // Turns cascade via FK ON DELETE CASCADE. `agent_runs.session_id` is a plain
    // (non-FK) column, so detach it to avoid dangling references.
    let _ = sqlx::query("UPDATE agent_runs SET session_id = NULL WHERE session_id = $1")
        .bind(sid)
        .execute(pool)
        .await;
    match sqlx::query("DELETE FROM agent_sessions WHERE session_id = $1")
        .bind(sid)
        .execute(pool)
        .await
    {
        Ok(r) if r.rows_affected() > 0 => {
            Json(json!({"deleted": true, "session_id": sid.to_string()})).into_response()
        }
        Ok(_) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "session not found"})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/agent/runs/:run_id
// ---------------------------------------------------------------------------

async fn run_lookup_handler(
    State(state): State<AgentState>,
    Path(run_id): Path<String>,
) -> Response {
    let uid = match uuid::Uuid::parse_str(&run_id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid run_id (expected uuid)"})),
            )
                .into_response();
        }
    };
    // Local degraded mode: serve the dreaming agent-run trace from the store.
    // The conversation drilldown (`/v1/agent/runs/:agent_run_id`) reads the
    // trace recorded inline by the dreaming run.
    if let Some(store) = state.local_dreaming.as_ref() {
        return match store.get_agent_run(uid) {
            Some((run, trace)) => {
                let started = run.get("started_at").cloned().unwrap_or(Value::Null);
                let finished = run.get("finished_at").cloned().unwrap_or(Value::Null);
                let model = run.get("model").cloned().unwrap_or(Value::Null);
                let status = run.get("status").cloned().unwrap_or(Value::Null);
                Json(json!({
                    "run_id": run_id,
                    "question": run.get("mode").and_then(|m| m.as_str())
                        .map(|m| format!("Dreaming run ({m})")).unwrap_or_else(|| "Dreaming run".into()),
                    "model_id": model,
                    "status": status,
                    "started_at": started,
                    "finished_at": finished,
                    "usage": run.get("stats").cloned().unwrap_or(Value::Null),
                    "trace": trace,
                }))
                .into_response()
            }
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": "run not found", "run_id": run_id})),
            )
                .into_response(),
        };
    }
    let Some(pool) = state.pool.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "run not found"})),
        )
            .into_response();
    };

    let row: Option<(
        String,
        String,
        String,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
        SqlxJson<Value>,
        SqlxJson<Value>,
    )> = match sqlx::query_as(
        r#"
        SELECT question, model_id, status, started_at, finished_at,
               usage_json, trace_json
        FROM agent_runs
        WHERE run_id = $1
        "#,
    )
    .bind(uid)
    .fetch_optional(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(run_id = %run_id, error = %e, "agent_runs lookup failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let Some((question, model, status, started_at, finished_at, usage, trace)) = row else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "run not found", "run_id": run_id})),
        )
            .into_response();
    };
    Json(json!({
        "run_id": run_id,
        "question": question,
        "model_id": model,
        "status": status,
        "started_at": started_at,
        "finished_at": finished_at,
        "usage": usage.0,
        "trace": trace.0,
    }))
    .into_response()
}

// ---------------------------------------------------------------------------
// Engine management routes
// ---------------------------------------------------------------------------

/// `GET /engines` — list available providers + their default models + the active config.
async fn list_engines(
    State(state): State<AgentState>,
) -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    // Nothing persisted yet (fresh install) is a normal state, not an error —
    // return the provider catalogue with a default-shaped `active` so the
    // settings UI can render the form and save a first config.
    let (active, configured) = match state.engines.get().await {
        Ok(a) => (a, true),
        Err(_) => (
            EngineConfig {
                kind: EngineKind::Ollama,
                model: String::new(),
                credential_id: None,
                host: None,
                extras: serde_json::Value::Null,
            },
            false,
        ),
    };
    // Pass the active Ollama host (if configured) so the catalogue's live
    // `/api/tags` fetch hits the user's actual instance, not the default.
    let ollama_hint = active.host.as_deref();
    let catalogue = engine_catalogue(ollama_hint).await;
    Ok(axum::Json(serde_json::json!({
        "available": catalogue,
        "active": active,
        "configured": configured,
    })))
}

/// `GET /engine` — the persisted EngineConfig (one global row, v1).
async fn get_engine(
    State(state): State<AgentState>,
) -> Result<axum::Json<EngineConfig>, (axum::http::StatusCode, String)> {
    state
        .engines
        .get()
        .await
        .map(axum::Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// `PUT /engine` — save a new EngineConfig.
async fn put_engine(
    State(state): State<AgentState>,
    axum::Json(cfg): axum::Json<EngineConfig>,
) -> Result<axum::Json<EngineConfig>, (axum::http::StatusCode, String)> {
    state
        .engines
        .put(&cfg)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(cfg))
}

/// `POST /engine/test` — dry-run probe against a candidate config without
/// persisting it. Confirms creds resolve + the provider responds.
async fn test_engine(
    State(state): State<AgentState>,
    axum::Json(cfg): axum::Json<EngineConfig>,
) -> Result<axum::Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    use adk_rust::futures::StreamExt;
    use adk_rust::{GenerateContentConfig, LlmRequest};

    // Claude CLI engine: spawn the binary with a 1-token prompt and confirm
    // it streams *any* text and completes without error. Bypasses build_engine
    // entirely because the CLI doesn't go through adk-rust.
    if cfg.kind == EngineKind::ClaudeCli {
        let probe = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            let mut rx = claude_cli::run_stream("ping", Some(&cfg.model), None, None, None)
                .map_err(|e| format!("spawn: {e}"))?;
            let mut got_output = false;
            let mut err: Option<String> = None;
            while let Some(ev) = rx.recv().await {
                match ev {
                    claude_cli::ClaudeEvent::TextStart { .. }
                    | claude_cli::ClaudeEvent::TextDelta { .. } => got_output = true,
                    claude_cli::ClaudeEvent::Error { message } => err = Some(message),
                    claude_cli::ClaudeEvent::Result { is_error: true, .. } => {
                        err.get_or_insert_with(|| "claude reported an error".to_string());
                    }
                    _ => {}
                }
            }
            if let Some(e) = err {
                return Err(e);
            }
            if !got_output {
                return Err("claude produced no output".to_string());
            }
            Ok::<(), String>(())
        })
        .await;

        return match probe {
            Ok(Ok(())) => Ok(axum::Json(serde_json::json!({
                "ok": true,
                "kind": cfg.kind,
                "model": cfg.model,
            }))),
            Ok(Err(msg)) => Err((axum::http::StatusCode::BAD_GATEWAY, msg)),
            Err(_elapsed) => Err((
                axum::http::StatusCode::GATEWAY_TIMEOUT,
                "claude_cli probe timed out after 30s".into(),
            )),
        };
    }

    // adk-rust path for Anthropic / OpenAI / Ollama.
    let resolver = CredentialResolver::new(state.credentials.clone(), state.tenant);
    let key = resolver.resolve(&cfg).await.map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            format!("credential: {e}"),
        )
    })?;
    let llm = build_engine(&cfg, key)
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, format!("init: {e}")))?;

    let req = LlmRequest {
        model: cfg.model.clone(),
        contents: vec![Content::new("user").with_text("ping")],
        config: Some(GenerateContentConfig {
            max_output_tokens: Some(1),
            ..Default::default()
        }),
        tools: Default::default(),
    };

    let probe = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        let mut stream = llm
            .generate_content(req, false)
            .await
            .map_err(|e| format!("provider: {e:?}"))?;
        while let Some(item) = stream.next().await {
            item.map_err(|e| format!("stream: {e:?}"))?;
            break;
        }
        Ok::<(), String>(())
    })
    .await;

    match probe {
        Ok(Ok(())) => Ok(axum::Json(serde_json::json!({
            "ok": true,
            "kind": cfg.kind,
            "model": cfg.model,
        }))),
        Ok(Err(msg)) => Err((axum::http::StatusCode::BAD_GATEWAY, msg)),
        Err(_elapsed) => Err((
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            "probe timed out after 30s".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Skill management routes
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
struct SkillRow {
    name: String,
    description: String,
    source: super::skills::SkillSource,
    path: String,
    enabled: bool,
    /// Truncated body preview for the UI tooltip.
    preview: String,
}

/// `GET /skills` — list every discovered skill on the host, plus its enabled
/// state. Discovery is cheap (filesystem walk), so we run it on every request
/// rather than caching.
async fn list_skills(
    State(state): State<AgentState>,
) -> Result<axum::Json<Vec<SkillRow>>, (axum::http::StatusCode, String)> {
    let enabled = state
        .skills
        .get()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let enabled_set: std::collections::HashSet<&str> = enabled.iter().map(String::as_str).collect();

    let mut rows: Vec<SkillRow> = super::skills::discover_all()
        .into_iter()
        .map(|s| {
            let preview = preview_body(&s.body);
            SkillRow {
                enabled: enabled_set.contains(s.name.as_str()),
                name: s.name,
                description: s.description,
                source: s.source,
                path: s.path,
                preview,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(axum::Json(rows))
}

/// `GET /skills/enabled` — just the toggled set, for callers that don't need
/// the full discovery payload.
async fn get_enabled_skills(
    State(state): State<AgentState>,
) -> Result<axum::Json<Vec<String>>, (axum::http::StatusCode, String)> {
    state
        .skills
        .get()
        .await
        .map(axum::Json)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Debug, serde::Deserialize)]
struct PutEnabledSkillsBody {
    skills: Vec<String>,
}

/// `PUT /skills/enabled` — replace the toggled set wholesale.
async fn put_enabled_skills(
    State(state): State<AgentState>,
    axum::Json(body): axum::Json<PutEnabledSkillsBody>,
) -> Result<axum::Json<Vec<String>>, (axum::http::StatusCode, String)> {
    state
        .skills
        .put(&body.skills)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(axum::Json(body.skills))
}

fn preview_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.chars().count() <= 200 {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(200).collect();
    out.push('…');
    out
}

// ---------------------------------------------------------------------------
// Claude CLI engine — drives the Claude Code agent loop via the
// claude-code-agent-sdk and translates its events into the UI Message Stream.
// Bypasses adk-rust entirely (Claude Code owns its own tool loop). Multi-turn
// context is preserved by resuming Claude's own session (`--resume`).
// ---------------------------------------------------------------------------

async fn ask_via_claude_cli(
    pool: Option<PgPool>,
    question: &str,
    cfg: &EngineConfig,
    resume_session_id: Option<String>,
    mcp_url: Option<String>,
    auth_header: Option<String>,
) -> Response {
    let run_id = uuid::Uuid::new_v4();
    let started_at = Utc::now();
    let start = Instant::now();
    let model = format!("claude_cli/{}", cfg.model);

    info!(run_id = %run_id, model = %model, resume = ?resume_session_id, mcp = mcp_url.is_some(), "claude_cli ask starting");

    let (ui, rx) = ui_stream::channel();
    ui.start(&run_id.to_string());
    ui.start_step();
    ui.data("model", json!({ "model": model }));

    let question_owned = question.to_string();
    let model_label = cfg.model.clone();
    tokio::spawn(async move {
        let mut answer = String::new();
        let mut errored: Option<String> = None;
        // Claude's own session id, echoed to the client so the next turn can
        // resume this conversation.
        let mut claude_session = String::new();
        let mut total_cost_usd: Option<f64> = None;
        let mut num_turns: u32 = 0;

        // Point the agent at our own MCP server so it can query the user's data.
        let mcp = mcp_url.map(|url| claude_cli::McpConfig {
            url,
            auth_header,
            strict: false,
        });

        let mut events = match claude_cli::run_stream(
            &question_owned,
            Some(&model_label),
            resume_session_id.as_deref(),
            None,
            mcp.as_ref(),
        ) {
            Ok(rx) => rx,
            Err(e) => {
                ui.error(&e.to_string());
                ui.data("usage", json!({ "run_id": run_id.to_string(), "elapsed_ms": start.elapsed().as_millis() as u64 }));
                ui.finish_step();
                ui.finish();
                ui.done();
                let _ = persist_run(
                    pool.as_ref(),
                    run_id,
                    &question_owned,
                    &model,
                    kyma_core::tenant::DEFAULT_TENANT.as_uuid(),
                    None,
                    started_at,
                    Utc::now(),
                    "error",
                    &Value::Null,
                    &json!([{ "event": "run_error", "data": { "message": e.to_string() } }]),
                )
                .await;
                return;
            }
        };

        while let Some(ev) = events.recv().await {
            match ev {
                claude_cli::ClaudeEvent::Init { session_id } => {
                    claude_session = session_id.clone();
                    ui.data("session", json!({ "sessionId": session_id }));
                }
                claude_cli::ClaudeEvent::TextStart { block_id } => ui.text_start(&block_id),
                claude_cli::ClaudeEvent::TextDelta { block_id, text } => {
                    answer.push_str(&text);
                    ui.text_delta(&block_id, &text);
                }
                claude_cli::ClaudeEvent::TextEnd { block_id } => ui.text_end(&block_id),
                claude_cli::ClaudeEvent::ThinkingStart { block_id } => {
                    ui.reasoning_start(&block_id)
                }
                claude_cli::ClaudeEvent::ThinkingDelta { block_id, text } => {
                    ui.reasoning_delta(&block_id, &text)
                }
                claude_cli::ClaudeEvent::ThinkingEnd { block_id } => ui.reasoning_end(&block_id),
                claude_cli::ClaudeEvent::ToolUse { id, name, input } => {
                    ui.tool_input_available(&id, &name, input)
                }
                claude_cli::ClaudeEvent::ToolResult {
                    id,
                    output,
                    is_error,
                } => {
                    if is_error {
                        let txt = output
                            .as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| output.to_string());
                        ui.tool_output_error(&id, &txt);
                    } else {
                        ui.tool_output_available(&id, output);
                    }
                }
                claude_cli::ClaudeEvent::Result {
                    session_id,
                    total_cost_usd: cost,
                    num_turns: turns,
                    is_error,
                    ..
                } => {
                    if claude_session.is_empty() {
                        claude_session = session_id;
                    }
                    total_cost_usd = cost;
                    num_turns = turns;
                    if is_error {
                        errored.get_or_insert_with(|| "claude reported an error".to_string());
                    }
                }
                claude_cli::ClaudeEvent::Error { message } => {
                    warn!(run_id = %run_id, message = %message, "claude_cli error");
                    ui.error(&message);
                    errored = Some(message);
                }
            }
        }

        ui.data(
            "usage",
            json!({
                "run_id": run_id.to_string(),
                "elapsed_ms": start.elapsed().as_millis() as u64,
                "total_cost_usd": total_cost_usd,
                "num_turns": num_turns,
                "session_id": claude_session,
            }),
        );
        ui.finish_step();
        ui.finish();
        ui.done();

        let _ = persist_run(
            pool.as_ref(),
            run_id,
            &question_owned,
            &model,
            kyma_core::tenant::DEFAULT_TENANT.as_uuid(),
            None,
            started_at,
            Utc::now(),
            if errored.is_some() {
                "error"
            } else {
                "success"
            },
            &Value::String(answer.clone()),
            &json!([{ "event": "answer_final", "data": { "text": answer } }]),
        )
        .await;
    });

    ui_stream::response(rx)
}
