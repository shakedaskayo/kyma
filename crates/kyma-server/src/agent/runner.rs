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
    SharedToolCtx, tool_describe_table, tool_list_databases, tool_run_sql, tool_sample_rows,
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

const SYSTEM_PROMPT: &str = r#"You are kyma's data assistant. The user will ask questions in English about their data. Use the tools to answer.

Recipe:
1. If the user hasn't named a specific database or table, first call list_databases and describe_table on candidates.
2. Once you know the schema, write a SQL query and call run_sql. Kyma's SQL dialect is DataFusion SQL; for vector similarity use cosine_distance(col, make_array(...)).
3. If a column's shape is unclear from describe_table, call sample_rows for one or two example records.
4. Produce a concise final answer in plain English. Cite the SQL you ran.

Rules:
- Do NOT fabricate schema. Always verify via describe_table before writing SQL.
- Do NOT claim data you didn't fetch via a tool call.
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
        .description("Kyma inline data assistant — answers English questions about the user's data.")
        .instruction(SYSTEM_PROMPT)
        .model(llm)
        .tool(tool_list_databases(shared.clone()))
        .tool(tool_describe_table(shared.clone()))
        .tool(tool_run_sql(shared.clone()))
        .tool(tool_sample_rows(shared))
        .build()?;

    Ok(Arc::new(agent))
}

/// Construct a `Runner` and create a fresh in-memory session bound to
/// `session_id`. Returns the runner + the session id (unchanged) so the
/// caller can drive `runner.run(...)` next.
pub async fn make_runner(
    state: &AgentState,
    session_id: &str,
) -> adk_rust::Result<Runner> {
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
