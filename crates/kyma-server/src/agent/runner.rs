//! ADK-Rust `Runner` wiring for the kyma inline data-assistant.
//!
//! Builds a fresh agent backed by the configured engine (Ollama, Anthropic, or
//! OpenAI) with the inline tools from [`super::tools`] and returns a [`Runner`]
//! that the HTTP layer drives via SSE.

use adk_rust::agent::LlmAgentBuilder;
use adk_rust::futures::StreamExt;
use adk_rust::identity::{SessionId, UserId};
use adk_rust::runner::{Runner, RunnerConfig};
use adk_rust::session::{CreateRequest, Event, InMemorySessionService, SessionService};
use adk_rust::{Agent, Content, Part};
use std::collections::HashMap;
use std::sync::Arc;

use crate::agent::engine::{build_engine, CredentialResolver};

use super::memory_tools::{
    tool_flush_memory, tool_ingest_entity, tool_link_memory_to_entity, tool_list_memories,
    tool_memory_compare, tool_memory_judge, tool_memory_search, tool_memory_session_summary,
    tool_merge_memories, tool_recall_memory, tool_reinforce_memory, tool_save_memories,
    tool_save_memory, tool_update_memory_importance, tool_update_memory_status,
};
use super::sessions::Turn;
use super::state::AgentState;
use super::tools::{
    tool_describe_table, tool_explore_schema, tool_find_references_to, tool_graph_traverse,
    tool_list_databases, tool_run_kql, tool_run_sql, tool_sample_rows, SharedToolCtx,
};

/// Application name advertised to the session service. Stable so that
/// session ids hash consistently across turns (once we add session reuse).
pub const APP_NAME: &str = "kyma-agent";

/// Stable agent name. The runner filters conversation history by author, so
/// seeded assistant turns must use this exact name to be replayed (see
/// [`make_runner`]).
pub const AGENT_NAME: &str = "kyma-assistant";

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

MEMORY — you have a persistent memory across sessions:
- `memory_search(query)` — the PRIMARY recall tool. Graph-aware hybrid search (semantic + keyword) expanded over connected memories, catalog resources, and traces. Call this early when a question may depend on prior context, preferences, or how entities relate. Returns ranked memories + a `linked` list of connected resources + a ready-to-use context block. Follow `linked` node ids with `graph_traverse` for a deeper subgraph. (`recall_memory` is an alias.)
- `save_memory(content, memory_type, …)` — store durable facts/decisions/preferences/learnings the user shares. Link them to entities with `link_memory_to_entity` when they're about a specific repo/service/table.
- `ingest_entity(name, kind, properties, links, type, icon)` — mint a virtual graph entity and wire it to nodes/memories. Prefer a `type` of the form `provider::resource` (e.g. `kubernetes::pod`, `aws::ec2::instance`, `github::repository`, `datadog::monitor`) — it sets the right brand mark AND resource classification. If there's no provider, set `icon` to a gallery name (a brand like github/datadog/kubernetes, or a kind glyph like service/repo/database/person/infra).
- Don't save trivia, transient state, or things already in the data. Prefer recalling before answering over guessing.

Rules:
- Do NOT fabricate schema. Always verify via a tool before writing a query.
- Do NOT claim data you didn't fetch via a tool call.
- Prefer ONE multi-tool turn over several sequential single-tool turns when calls are independent.
- You have at most 12 tool calls per question.
"#;

/// Compose the agent's system prompt: the base [`SYSTEM_PROMPT`] constant
/// plus a block listing every enabled skill the user has toggled on. The
/// skill block is what makes "Settings → Skills" load-bearing — the agent
/// only sees skills that appear here.
async fn compose_system_prompt(state: &AgentState) -> String {
    // Base prompt + the live icon gallery (so agent-minted entities get a
    // recognisable mark, and external gallery config is reflected here).
    let base = format!(
        "{SYSTEM_PROMPT}\nICON GALLERY — when calling `ingest_entity`, pick `icon` from: {}.\n",
        crate::icon_config::IconGallery::global().catalog_hint(),
    );

    let enabled = match state.skills.get().await {
        Ok(s) => s,
        Err(_) => return base,
    };
    if enabled.is_empty() {
        return base;
    }
    let enabled_set: std::collections::HashSet<&str> = enabled.iter().map(String::as_str).collect();
    let discovered = crate::agent::skills::discover_all();
    let mut active: Vec<_> = discovered
        .into_iter()
        .filter(|s| enabled_set.contains(s.name.as_str()))
        .collect();
    if active.is_empty() {
        return base;
    }
    active.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = base;
    out.push_str(
        "\n\n---\nADDITIONAL SKILLS — the user has enabled these. Use them when they apply.\n",
    );
    for s in active {
        out.push_str(&format!("\n## Skill: {}\n", s.name));
        if !s.description.is_empty() {
            out.push_str(&format!("**When to use:** {}\n\n", s.description));
        }
        out.push_str(s.body.trim());
        out.push_str("\n");
    }
    out
}

/// Build a fresh agent backed by the configured engine and wired with the
/// inline tools. Async now — the engine store and credential store are both
/// IO-bound.
pub async fn build_agent(state: &AgentState) -> anyhow::Result<Arc<dyn Agent>> {
    let cfg = state.engines.get().await?;
    let resolver = CredentialResolver::new(state.credentials.clone(), state.tenant);
    let key = resolver.resolve(&cfg).await?;
    let llm = build_engine(&cfg, key)?;

    let shared = SharedToolCtx {
        federation: Some(kyma_federation::runtime_from(state.credentials.clone())),
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
        memory: state.memory.clone(),
        hitl: None,
        memory_settings_path: state.memory_settings_path.clone(),
    };

    let agent = LlmAgentBuilder::new(AGENT_NAME)
        .description(
            "Kyma inline data assistant — answers English questions about the user's data.",
        )
        .instruction(compose_system_prompt(state).await)
        .model(llm)
        .tool(tool_list_databases(shared.clone()))
        .tool(tool_explore_schema(shared.clone()))
        .tool(tool_describe_table(shared.clone()))
        .tool(tool_run_kql(shared.clone()))
        .tool(tool_run_sql(shared.clone()))
        .tool(tool_sample_rows(shared.clone()))
        .tool(tool_find_references_to(shared.clone()))
        .tool(tool_graph_traverse(shared.clone()))
        // Agentic Memory tools.
        .tool(tool_save_memory(shared.clone()))
        .tool(tool_save_memories(shared.clone()))
        .tool(tool_flush_memory(shared.clone()))
        .tool(tool_memory_search(shared.clone()))
        .tool(tool_recall_memory(shared.clone()))
        .tool(tool_list_memories(shared.clone()))
        .tool(tool_link_memory_to_entity(shared.clone()))
        .tool(tool_ingest_entity(shared.clone()))
        .tool(tool_update_memory_status(shared.clone()))
        .tool(tool_update_memory_importance(shared.clone()))
        .tool(tool_merge_memories(shared.clone()))
        .tool(tool_memory_compare(shared.clone()))
        .tool(tool_memory_judge(shared.clone()))
        .tool(tool_memory_session_summary(shared.clone()))
        // Usage-based reinforcement (M8.1): report whether a recalled memory
        // actually helped so recall can learn from it.
        .tool(tool_reinforce_memory(shared))
        .build()
        .map_err(|e| anyhow::anyhow!("agent build failed: {e:?}"))?;

    Ok(Arc::new(agent))
}

/// Construct a `Runner` and create a fresh in-memory session bound to
/// `session_id`, seeded with any prior conversation `history` (and an optional
/// rolling `summary` of older turns) so follow-ups carry context. Returns the
/// runner so the caller can drive `runner.run(...)` next.
pub async fn make_runner(
    state: &AgentState,
    session_id: &str,
    history: &[Turn],
    summary: Option<&str>,
) -> anyhow::Result<Runner> {
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

    seed_history(&sessions, session_id, history, summary).await;

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

/// Replay prior turns into the in-memory session as ADK events. The runner
/// builds conversation history from session events, mapping role by
/// `(author, content.role)`: author `"user"` → user; otherwise → model. So a
/// user turn uses author `"user"`, and an assistant turn uses author
/// [`AGENT_NAME`] (matching the live agent) + content role `"model"`, which is
/// what keeps it in the replayed history.
async fn seed_history(
    sessions: &Arc<dyn SessionService>,
    session_id: &str,
    history: &[Turn],
    summary: Option<&str>,
) {
    if let Some(s) = summary {
        let s = s.trim();
        if !s.is_empty() {
            let mut ev = Event::new("seed-summary");
            ev.author = "user".to_string();
            ev.set_content(Content::new("user").with_text(format!(
                "Summary of earlier conversation (for context):\n{s}"
            )));
            let _ = sessions.append_event(session_id, ev).await;
        }
    }
    for (i, turn) in history.iter().enumerate() {
        if turn.text.trim().is_empty() {
            continue;
        }
        let is_assistant = turn.role == "assistant";
        let mut ev = Event::new(format!("seed-{i}"));
        ev.author = if is_assistant {
            AGENT_NAME.to_string()
        } else {
            "user".to_string()
        };
        let role = if is_assistant { "model" } else { "user" };
        ev.set_content(Content::new(role).with_text(turn.text.clone()));
        let _ = sessions.append_event(session_id, ev).await;
    }
}

/// Run a one-shot, tool-less LLM turn with the configured engine: build a
/// minimal agent carrying `instruction`, feed `input` as the user message, and
/// return the model's final text. Shared by the rolling-summary pass and the
/// memory-extraction pipeline (which parses the returned text as JSON).
///
/// Errors if the configured engine is `claude_cli` (it doesn't run through
/// adk-rust) — callers that need a fallback should check the engine kind first.
pub async fn run_oneshot(
    state: &AgentState,
    agent_name: &str,
    description: &str,
    instruction: &str,
    input: &str,
) -> anyhow::Result<String> {
    let cfg = state.engines.get().await?;
    let resolver = CredentialResolver::new(state.credentials.clone(), state.tenant);
    let key = resolver.resolve(&cfg).await?;
    let llm = build_engine(&cfg, key)?;
    let agent = LlmAgentBuilder::new(agent_name)
        .description(description)
        .instruction(instruction)
        .model(llm)
        .build()
        .map_err(|e| anyhow::anyhow!("oneshot agent build failed: {e:?}"))?;
    let agent: Arc<dyn Agent> = Arc::new(agent);

    let sessions: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    let sid = uuid::Uuid::new_v4().to_string();
    sessions
        .create(CreateRequest {
            app_name: APP_NAME.to_string(),
            user_id: ANON_USER.to_string(),
            session_id: Some(sid.clone()),
            state: HashMap::new(),
        })
        .await
        .map_err(|e| anyhow::anyhow!("oneshot session create failed: {e:?}"))?;

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
    .map_err(|e| anyhow::anyhow!("oneshot runner build failed: {e:?}"))?;

    let user_id = UserId::new(ANON_USER).map_err(|e| anyhow::anyhow!("user_id: {e}"))?;
    let session_id = SessionId::new(&sid).map_err(|e| anyhow::anyhow!("session_id: {e}"))?;
    let content = Content::new("user").with_text(input.to_string());

    let mut stream = runner
        .run(user_id, session_id, content)
        .await
        .map_err(|e| anyhow::anyhow!("oneshot run: {e:?}"))?;

    let mut out = String::new();
    while let Some(ev) = stream.next().await {
        let ev = ev.map_err(|e| anyhow::anyhow!("oneshot event: {e:?}"))?;
        if ev.llm_response.partial {
            continue;
        }
        for c in ev.llm_response.content.iter() {
            for part in c.parts.iter() {
                if let Part::Text { text } = part {
                    out.push_str(text);
                }
            }
        }
    }
    Ok(out.trim().to_string())
}

/// Run a one-shot summarization of `transcript` with the configured engine and
/// return the model's final text. Used by the rolling-summary pass.
pub async fn summarize_conversation(
    state: &AgentState,
    transcript: &str,
) -> anyhow::Result<String> {
    run_oneshot(
        state,
        "kyma-summarizer",
        "Summarizes prior conversation for context compression.",
        "You compress conversations. Given a transcript, return a concise summary (3-6 \
         sentences) that preserves durable facts, decisions, and the user's intent. \
         Return only the summary text — no preamble, no headings.",
        transcript,
    )
    .await
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
