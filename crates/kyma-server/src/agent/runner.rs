//! ADK-Rust `Runner` wiring for the kyma inline data-assistant.
//!
//! Builds a fresh Ollama-backed [`LlmAgent`](adk_rust::agent::LlmAgent) with the
//! four inline tools from [`super::tools`] and returns a [`Runner`] + session
//! id that the HTTP layer drives via SSE.

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::model::ollama::{OllamaConfig, OllamaModel};
use adk_rust::runner::{Runner, RunnerConfig};
use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};
use adk_rust::{Agent, Llm};
use std::collections::HashMap;
use std::sync::Arc;

use super::state::AgentState;
use super::tools::{
    tool_describe_table, tool_list_databases, tool_run_kql, tool_run_sql, tool_sample_rows,
    SharedToolCtx,
};

/// Application name advertised to the session service. Stable so that
/// session ids hash consistently across turns (once we add session reuse).
pub const APP_NAME: &str = "kyma-agent";

/// User id used when the endpoint is hit without an authenticated subject.
/// Matches the stub `auth_subject` column written into `agent_runs`.
pub const ANON_USER: &str = "anonymous";

/// Default Ollama model — confirmed loaded on this host via `/api/tags`.
pub const DEFAULT_MODEL: &str = "gemma4:latest";
/// Default Ollama server URL (matches the host's running daemon).
pub const DEFAULT_OLLAMA_HOST: &str = "http://localhost:11434";

const SYSTEM_PROMPT: &str = r#"You are kyma's data assistant. Users ask questions in English; you answer them by using the tools.

KQL IS THE PRIMARY QUERY LANGUAGE — prefer `run_kql` over `run_sql`.

KQL uses pipe syntax. Examples:
- Counting:        `requests | where status >= 500 | summarize n=count() by url | top 10 by n`
- Time-bucket:     `requests | where ts > ago(1h) | summarize n=count() by bin(ts, 1m) | sort by ts asc`
- Text search:     `requests | where url contains "/api/" | take 20`
- Distinct values: `requests | distinct service`
- Graph:           `edges | graph-traverse source "a" from src to dst max-hops 3`
- Projection:      `requests | project ts, url, status | take 100`

Use `run_sql` only for: vector similarity search (cosine_distance(col, make_array(..)) UDF is SQL-only today), recursive CTEs (`WITH RECURSIVE`), window functions, or joins across many tables. `run_sql` is the escape hatch, not the default.

Efficient workflow:
1. If the user did not name a specific database or table, batch independent lookups in the same turn — emit BOTH `list_databases` AND `describe_table` calls in one response when feasible (the engine dispatches them in parallel).
2. Once you know the schema, write a KQL pipeline and call `run_kql`. For vector similarity, fall back to `run_sql` with `cosine_distance`.
3. If a column's shape is unclear from `describe_table`, call `sample_rows` for one or two example records.
4. Produce a concise final answer in plain English. Cite the KQL (or SQL) you ran.

Rules:
- Do NOT fabricate schema. Always verify via `describe_table` before writing a query.
- Do NOT claim data you didn't fetch via a tool call.
- Prefer ONE multi-tool turn over several sequential single-tool turns when the calls are independent.
- You have at most 12 tool calls per question.
"#;

/// Build a fresh agent backed by Ollama and wired with the four inline tools.
pub fn build_agent(state: &AgentState) -> adk_rust::Result<Arc<dyn Agent>> {
    let cfg = OllamaConfig {
        host: std::env::var("KYMA_AGENT_OLLAMA_HOST")
            .unwrap_or_else(|_| DEFAULT_OLLAMA_HOST.to_string()),
        model: std::env::var("KYMA_AGENT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string()),
        temperature: Some(0.0),
        num_ctx: None,
        top_p: None,
        top_k: None,
    };
    let llm: Arc<dyn Llm> = Arc::new(OllamaModel::new(cfg)?);

    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
    };

    let agent = LlmAgentBuilder::new("kyma-assistant")
        .description(
            "Kyma inline data assistant — answers English questions about the user's data.",
        )
        .instruction(SYSTEM_PROMPT)
        .model(llm)
        .tool(tool_list_databases(shared.clone()))
        .tool(tool_describe_table(shared.clone()))
        .tool(tool_run_kql(shared.clone()))
        .tool(tool_run_sql(shared.clone()))
        .tool(tool_sample_rows(shared))
        .build()?;

    Ok(Arc::new(agent))
}

/// Construct a `Runner` and create a fresh in-memory session bound to
/// `session_id`. Returns the runner + the session id (unchanged) so the
/// caller can drive `runner.run(...)` next.
pub async fn make_runner(state: &AgentState, session_id: &str) -> adk_rust::Result<Runner> {
    let agent = build_agent(state)?;

    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: APP_NAME.to_string(),
            user_id: ANON_USER.to_string(),
            session_id: Some(session_id.to_string()),
            state: HashMap::new(),
        })
        .await?;

    let runner = Runner::new(RunnerConfig {
        app_name: APP_NAME.to_string(),
        agent,
        session_service: sessions,
        artifact_service: None,
        memory_service: None,
        plugin_manager: None,
        run_config: None,
        compaction_config: None,
        context_cache_config: None,
        cache_capable: None,
        request_context: None,
        cancellation_token: None,
    })?;

    Ok(runner)
}

/// Effective model id string persisted into `agent_runs.model_id`.
pub fn model_id() -> String {
    let m = std::env::var("KYMA_AGENT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    format!("ollama/{m}")
}
