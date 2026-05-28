//! ADK-Rust `Runner` wiring for the kyma inline data-assistant.
//!
//! Builds a fresh agent backed by the configured engine (Ollama, Anthropic, or
//! OpenAI) with the inline tools from [`super::tools`] and returns a [`Runner`]
//! that the HTTP layer drives via SSE.

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::runner::{Runner, RunnerConfig};
use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};
use adk_rust::Agent;
use std::collections::HashMap;
use std::sync::Arc;

use crate::agent::engine::{build_engine, CredentialResolver};

use super::state::AgentState;
use super::tools::{
    tool_describe_table, tool_explore_schema, tool_find_references_to, tool_graph_traverse,
    tool_list_databases, tool_run_kql, tool_run_sql, tool_sample_rows, SharedToolCtx,
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

Cross-entity questions — USE THE GRAPH TOOLS:
- Start with `explore_schema(database)` for a one-shot view of every table + columns + sample values. Much cheaper than many `describe_table` calls. USE THIS FIRST when the question touches multiple tables or you don't yet know how entities relate.
- `find_references_to(value)` — given a value like "user-42", returns every (database, table, column) where it appears. The "where else does this show up?" primitive.
- `graph_traverse(database, edges_table, source, from_column, to_column, max_hops)` — wraps KQL's graph-traverse for tables that store edges as rows. Use for connectivity: "what services depend on X?".

Efficient workflow:
1. Open with `explore_schema` (full database view) OR `list_databases` + `describe_table` on specific targets. Batch independent lookups in the SAME turn — emit multiple tool calls together and the engine dispatches them in parallel.
2. For relationship questions, prefer `find_references_to` / `graph_traverse` over hand-written joins.
3. Once you know the schema, write a KQL pipeline and call `run_kql`. For vector similarity, fall back to `run_sql` with `cosine_distance`.
4. If a column's shape is unclear, call `sample_rows` for a few example records.
5. Produce a concise final answer in plain English. Cite the KQL (or SQL) you ran.

Rules:
- Do NOT fabricate schema. Always verify via a tool before writing a query.
- Do NOT claim data you didn't fetch via a tool call.
- Prefer ONE multi-tool turn over several sequential single-tool turns when calls are independent.
- You have at most 12 tool calls per question.
"#;

/// Build a fresh agent backed by the configured engine and wired with the
/// inline tools. Async now — the engine store and credential store are both
/// IO-bound.
pub async fn build_agent(state: &AgentState) -> anyhow::Result<Arc<dyn Agent>> {
    let cfg = state.engines.get().await?;
    let resolver = CredentialResolver::new(state.credentials.clone(), state.tenant);
    let key = resolver.resolve(&cfg).await?;
    let llm = build_engine(&cfg, key)?;

    let shared = SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
    };

    let agent = LlmAgentBuilder::new("kyma-assistant")
        .description(
            "Kyma inline data assistant — answers English questions about the user's data.",
        )
        .instruction(SYSTEM_PROMPT)
        .model(llm)
        .tool(tool_list_databases(shared.clone()))
        .tool(tool_explore_schema(shared.clone()))
        .tool(tool_describe_table(shared.clone()))
        .tool(tool_run_kql(shared.clone()))
        .tool(tool_run_sql(shared.clone()))
        .tool(tool_sample_rows(shared.clone()))
        .tool(tool_find_references_to(shared.clone()))
        .tool(tool_graph_traverse(shared))
        .build()
        .map_err(|e| anyhow::anyhow!("agent build failed: {e:?}"))?;

    Ok(Arc::new(agent))
}

/// Construct a `Runner` and create a fresh in-memory session bound to
/// `session_id`. Returns the runner + the session id (unchanged) so the
/// caller can drive `runner.run(...)` next.
pub async fn make_runner(state: &AgentState, session_id: &str) -> anyhow::Result<Runner> {
    let agent = build_agent(state).await?;

    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    sessions
        .create(CreateRequest {
            app_name: APP_NAME.to_string(),
            user_id: ANON_USER.to_string(),
            session_id: Some(session_id.to_string()),
            state: HashMap::new(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("session create failed: {e:?}"))?;

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
    })
    .map_err(|e| anyhow::anyhow!("runner build failed: {e:?}"))?;

    Ok(runner)
}

/// Effective model id string persisted into `agent_runs.model_id`. Reads the
/// active engine config; if the store is unreachable for any reason, returns
/// the legacy Ollama default so the row insert never fails.
pub async fn model_id(state: &AgentState) -> String {
    match state.engines.get().await {
        Ok(cfg) => format!("{}/{}", cfg.kind.as_str(), cfg.model),
        Err(_) => format!("ollama/{}", DEFAULT_MODEL),
    }
}
