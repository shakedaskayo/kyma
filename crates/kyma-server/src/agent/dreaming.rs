//! Dreaming — scheduled agentic memory housekeeping.
//!
//! A dreaming run is an autonomous agent run (the configured engine: adk
//! providers or the Claude CLI) that reviews recent raw material, fills gaps
//! with READ-ONLY connector access, and housekeeps the memory store:
//! re-scores importance, builds relationships, dedups/merges, and archives
//! stale memories — all bi-temporal, never hard-deleting.
//!
//! Execution rides the worker fabric: [`DreamingScheduler`] enqueues a
//! `dreaming` job when enabled (OFF by default); the worker's executor calls
//! [`run_dreaming`]. Every run records:
//! - a `memory_pipeline_runs` row (`kind='dreaming'`) with outcome stats,
//! - an `agent_sessions` row (`source='dreaming'`) + turns,
//! - an `agent_runs` row whose `trace_json` carries the full tool-call
//!   trace — the conversation the UI drills into,
//! - a live progress snapshot on the fabric job (activity feed).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_rust::futures::StreamExt;
use adk_rust::identity::{SessionId, UserId};
use adk_rust::{Content, Part, Tool, ToolContext};
use chrono::Utc;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use kyma_core::tenant::TenantId;

use super::connector_tools::{
    tool_connector_read, tool_list_connectors, ConnectorReadBudget, ConnectorToolCtx,
};
use super::engine::{claude_cli, EngineKind};
use super::memory_settings::{self, DreamingSettings};
use super::routes::persist_run;
use super::sessions;
use super::state::AgentState;

/// Push one progress snapshot to wherever the run is being watched (the
/// fabric job's `progress` JSONB). Failures are observability-only.
pub type ProgressFn = Arc<dyn Fn(Value) -> BoxFuture<'static, ()> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Scheduled,
    Manual,
}

impl Trigger {
    fn as_str(&self) -> &'static str {
        match self {
            Trigger::Scheduled => "scheduled",
            Trigger::Manual => "manual",
        }
    }
}

/// One dreaming job's request (the fabric job payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamingRequest {
    pub trigger: Trigger,
    /// `full` | `housekeeping_only` | `sources`; falls back to settings.
    #[serde(default)]
    pub mode: Option<String>,
    /// Optional focus hint folded into the prompt (a realm, a connector, …).
    #[serde(default)]
    pub focus: Option<String>,
    /// The fabric job id (for run linkage); `None` when run inline.
    #[serde(default)]
    pub job_id: Option<Uuid>,
    /// The lease-holding worker (for run linkage).
    #[serde(default)]
    pub worker_id: Option<Uuid>,
}

/// Outcome counters persisted into `memory_pipeline_runs.stats_json`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DreamingOutcome {
    pub memories_created: u32,
    pub memories_merged: u32,
    pub memories_archived: u32,
    pub importance_rescored: u32,
    pub judgements: u32,
    pub entities_linked: u32,
    pub connector_reads: u32,
    pub tool_calls: u32,
    pub summary: String,
}

// ── live progress (activity feed) ────────────────────────────────────────────

const ACTIVITY_RING: usize = 20;

/// Builds the progress snapshot the UI renders live: a rolling activity feed
/// plus the current phase and counters. The executor owns it and pushes whole
/// snapshots through the [`ProgressFn`].
struct Activity {
    push: ProgressFn,
    ring: VecDeque<Value>,
    phase: String,
    outcome_snapshot: Value,
    thinking: String,
}

impl Activity {
    fn new(push: ProgressFn) -> Self {
        Self {
            push,
            ring: VecDeque::with_capacity(ACTIVITY_RING),
            phase: "starting".into(),
            outcome_snapshot: json!({}),
            thinking: String::new(),
        }
    }

    async fn event(&mut self, icon: &str, text: impl Into<String>) {
        let text: String = text.into().chars().take(200).collect();
        if self.ring.len() == ACTIVITY_RING {
            self.ring.pop_front();
        }
        self.ring
            .push_back(json!({ "icon": icon, "text": text, "ts": Utc::now().to_rfc3339() }));
        self.flush().await;
    }

    async fn phase(&mut self, phase: &str) {
        self.phase = phase.to_string();
        self.flush().await;
    }

    fn thinking(&mut self, delta: &str) {
        self.thinking.push_str(delta);
        if self.thinking.chars().count() > 240 {
            let tail: String = self
                .thinking
                .chars()
                .rev()
                .take(240)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            self.thinking = tail;
        }
        // No flush — thinking rides along with the next event/phase push.
    }

    fn counters(&mut self, outcome: &DreamingOutcome) {
        self.outcome_snapshot = json!({
            "memories_created": outcome.memories_created,
            "memories_merged": outcome.memories_merged,
            "memories_archived": outcome.memories_archived,
            "entities_linked": outcome.entities_linked,
            "connector_reads": outcome.connector_reads,
            "tool_calls": outcome.tool_calls,
        });
    }

    fn snapshot(&self) -> Value {
        json!({
            "current_phase": self.phase,
            "activity": self.ring.iter().cloned().collect::<Vec<_>>(),
            "thinking": if self.thinking.is_empty() { Value::Null } else { json!(self.thinking) },
            "counters": self.outcome_snapshot,
        })
    }

    async fn flush(&self) {
        (self.push)(self.snapshot()).await;
    }
}

// ── mutation budget (decorator over the memory write tools) ─────────────────

/// Shared mutation counter for one run. When the cap is hit, mutating tools
/// return an error payload so the agent stops mutating but can still produce
/// its summary.
pub struct MutationBudget {
    cap: u32,
    used: AtomicU32,
}

impl MutationBudget {
    fn new(cap: u32) -> Self {
        Self {
            cap,
            used: AtomicU32::new(0),
        }
    }
    fn take(&self) -> bool {
        self.used.fetch_add(1, Ordering::Relaxed) < self.cap
    }
}

/// Delegating wrapper that gates `execute` on the run's [`MutationBudget`].
struct BudgetedTool {
    inner: Arc<dyn Tool>,
    budget: Arc<MutationBudget>,
}

#[adk_rust::async_trait]
impl Tool for BudgetedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn declaration(&self) -> Value {
        self.inner.declaration()
    }
    fn is_long_running(&self) -> bool {
        self.inner.is_long_running()
    }
    fn parameters_schema(&self) -> Option<Value> {
        self.inner.parameters_schema()
    }
    fn response_schema(&self) -> Option<Value> {
        self.inner.response_schema()
    }
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }
    fn is_concurrency_safe(&self) -> bool {
        self.inner.is_concurrency_safe()
    }
    async fn execute(
        &self,
        ctx: Arc<dyn ToolContext>,
        args: Value,
    ) -> adk_rust::Result<Value> {
        if !self.budget.take() {
            return Ok(json!({"error": format!(
                "mutation budget exhausted for this dreaming run (cap {}) — stop mutating and \
                 write your summary report",
                self.budget.cap
            )}));
        }
        self.inner.execute(ctx, args).await
    }
}

// ── prompt ───────────────────────────────────────────────────────────────────

fn dreaming_prompt(
    mode: &str,
    focus: Option<&str>,
    realm_scope: &[String],
    s: &DreamingSettings,
) -> String {
    let scope = if realm_scope.is_empty() {
        "all realms".to_string()
    } else {
        realm_scope.join(", ")
    };
    let focus_line = focus
        .map(|f| format!("\nFOCUS for this run: {f}\n"))
        .unwrap_or_default();
    let gap_fill = mode != "housekeeping_only";
    let housekeeping = mode != "sources";

    let mut p = format!(
        "You are kyma's Dreaming agent — an autonomous background process that housekeeps the \
user's long-term memory store. Nobody is watching live; your final message becomes the run \
summary shown in the UI. Work in PHASES.{focus_line}
Scope: {scope}.

PHASE 1 — REVIEW recent raw material:
- Survey recent memories with memory_search / list_memories (scope above).
- Survey new raw activity: run_kql/run_sql over the `claude_code_events` firehose in the \
`default` database (recent sessions, what the user worked on) and any synced Claude Code \
memory files already in the memory store.
"
    );
    if gap_fill {
        p.push_str(&format!(
            "
PHASE 2 — GAP-FILL (budget: {} connector reads, READ-ONLY):
- When a memory references something with missing or stale context, use `list_connectors` \
then `connector_read` to fetch fresh context from the source (a GitHub README/file/issue, a \
SELECT against a connected Postgres).
- Save what you learn with save_memory and wire it with link_memory_to_entity / ingest_entity.
- Do not exceed the budget; if a read fails, move on.
",
            s.connector_read_budget
        ));
    }
    if housekeeping {
        p.push_str(&format!(
            "
PHASE 3 — GRAPH WIRING & ENTITY MAINTENANCE (the core of dreaming):
The context graph has three layers you must keep fully wired together:
(a) MEMORIES (the memory graph), (b) DETERMINISTIC RESOURCES — connector-ingested nodes \
(repos, files, issues, tables, services) living in their own database/graph namespaces, and \
(c) LOGICAL ENTITIES — virtual nodes you create for things that exist conceptually (a service, \
a person, a project, an architecture concept) but have no single deterministic row.
- For each significant memory, find what it is ABOUT: use find_references_to(value) and \
graph_traverse over the connector graphs to locate the deterministic node(s), then \
link_memory_to_entity(memory_id, target_node_id, target_namespace) — the namespace is the \
resource's `database/graph` (e.g. a github repo node lives in its connector's graph). A memory \
without edges is a dead memory.
- CREATE logical entities with ingest_entity for recurring concepts that deserve a node: \
prefer `type` as `provider::resource` (e.g. `github::repository`, `kubernetes::pod`) or a \
kind (service|repo|person|concept|config). Wire its `links` to the deterministic resources it \
abstracts over AND to the memories about it (\"memory:<uuid>\").
- MAINTAIN existing entities: ingest_entity is idempotent on (realm, kind, name) — re-ingest \
with refreshed properties/links when understanding evolves; never mint near-duplicate entities \
(search first with memory_search / find_references_to).
- Relate entities to each other with meaningful relationship_type values (DEPENDS_ON, OWNS, \
PART_OF) instead of leaving everything RELATES_TO.
- Re-score importance with update_memory_importance using these bands: critical operational \
knowledge 0.9+, team preferences/decisions 0.6–0.8, historical context 0.3–0.5, trivial 0.1–0.2.
- Deduplicate: memory_compare suspected duplicates, then merge_memories (keep the richer one).
- Resolve contradictions: memory_judge with verdict `supersedes` — bi-temporal, never delete; \
use `related`/`conflicts` verdicts to record weaker relationships between memories.
- Archive stale/outdated memories with update_memory_status(status=archived) and a reason.
Mutation cap for this run: {} — spend it on wiring quality, not volume.
",
            s.mutation_cap
        ));
    }
    p.push_str(
        "
FINAL PHASE — SUMMARY: end with a concise report of what you reviewed, what you changed and \
why (cite memory ids), and anything that needs human attention. This is your last message.

RULES:
- NEVER hard-delete; archival and superseding are the only removal paths.
- Connector access is READ-ONLY; do not attempt writes against sources.
- Prefer a few high-value mutations over many speculative ones.
- If budgets run out, proceed to the summary.
",
    );
    p
}

// ── run record (memory_pipeline_runs, kind='dreaming') ──────────────────────

#[allow(clippy::too_many_arguments)]
async fn insert_run_row(
    pool: &sqlx::PgPool,
    run_id: Uuid,
    tenant: TenantId,
    req: &DreamingRequest,
    mode: &str,
    engine: &str,
    model: &str,
    session_id: Uuid,
    agent_run_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO memory_pipeline_runs \
         (id, tenant_id, kind, status, started_at, mode, trigger, job_id, worker_id, \
          session_id, agent_run_id, engine, model) \
         VALUES ($1, $2, 'dreaming', 'running', $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(run_id)
    .bind(tenant.as_uuid())
    .bind(Utc::now())
    .bind(mode)
    .bind(req.trigger.as_str())
    .bind(req.job_id)
    .bind(req.worker_id)
    .bind(session_id)
    .bind(agent_run_id)
    .bind(engine)
    .bind(model)
    .execute(pool)
    .await?;
    Ok(())
}

async fn finalize_run_row(
    pool: &sqlx::PgPool,
    run_id: Uuid,
    status: &str,
    error: Option<&str>,
    outcome: &DreamingOutcome,
    progress: Value,
) {
    let stats = serde_json::to_value(outcome).unwrap_or_default();
    let _ = sqlx::query(
        "UPDATE memory_pipeline_runs SET status=$2, finished_at=$3, error=$4, \
         memories_written=$5, stats_json=$6, progress_json=$7 WHERE id=$1",
    )
    .bind(run_id)
    .bind(status)
    .bind(Utc::now())
    .bind(error)
    .bind(outcome.memories_created as i64)
    .bind(stats)
    .bind(progress)
    .execute(pool)
    .await;
}

// ── the executor ─────────────────────────────────────────────────────────────

/// Run one dreaming job to completion. Returns the run id + outcome; all
/// persistence (run row, session, agent_runs trace) happens inside.
pub async fn run_dreaming(
    state: &AgentState,
    progress: ProgressFn,
    req: DreamingRequest,
) -> anyhow::Result<(Uuid, DreamingOutcome)> {
    let Some(pool) = state.pool.clone() else {
        anyhow::bail!("dreaming requires Postgres (no pool in local mode)");
    };
    let settings = memory_settings::load(Some(&pool), state.tenant).await.dreaming;
    let mode = req
        .mode
        .clone()
        .unwrap_or_else(|| settings.mode.clone());
    let engine_cfg = state.engines.get().await?;
    let engine = engine_cfg.kind.as_str().to_string();
    let model = engine_cfg.model.clone();

    let run_id = Uuid::new_v4();
    let agent_run_id = Uuid::new_v4();
    let tenant_uuid = state.tenant.as_uuid();

    // Mint the conversation session the UI drills into.
    let sctx = sessions::load_or_create(Some(&pool), None, tenant_uuid, "dreaming", "dreaming").await;
    let session_uuid = sctx.session_id;
    let title = format!("Dreaming · {}", Utc::now().format("%b %e %H:%M"));
    let _ = sqlx::query("UPDATE agent_sessions SET title = $2 WHERE session_id = $1")
        .bind(session_uuid)
        .bind(&title)
        .execute(&pool)
        .await;

    insert_run_row(
        &pool, run_id, state.tenant, &req, &mode, &engine, &model, session_uuid, agent_run_id,
    )
    .await?;

    let prompt_user = match (&req.focus, mode.as_str()) {
        (Some(f), _) => format!("Dreaming run ({mode}) — focus: {f}"),
        (None, m) => format!("Dreaming run ({m})"),
    };
    // run_id stays NULL here: agent_session_turns.run_id FKs agent_runs, and
    // that row is only inserted after the run completes (persist_run below).
    sessions::persist_turn(
        Some(&pool),
        session_uuid,
        tenant_uuid,
        sctx.next_turn_index,
        "user",
        &prompt_user,
        None,
    )
    .await;

    let mut activity = Activity::new(progress);
    activity.phase("reviewing").await;
    info!(run_id = %run_id, mode = %mode, engine = %engine, "dreaming run starting");

    let started_at = Utc::now();
    let start = Instant::now();
    let wall_clock = Duration::from_secs(settings.wall_clock_secs.max(30));
    let system_prompt = dreaming_prompt(&mode, req.focus.as_deref(), &settings.realm_scope, &settings);

    let mut outcome = DreamingOutcome::default();
    let mut trace: Vec<Value> = Vec::new();
    let run_result: Result<(), String> = if engine_cfg.kind == EngineKind::ClaudeCli {
        run_via_claude_cli(
            state,
            &engine_cfg.model,
            &system_prompt,
            &prompt_user,
            wall_clock,
            &mut activity,
            &mut outcome,
            &mut trace,
        )
        .await
    } else {
        run_via_adk(
            state,
            &settings,
            &mode,
            &system_prompt,
            &prompt_user,
            &session_uuid.to_string(),
            wall_clock,
            &mut activity,
            &mut outcome,
            &mut trace,
        )
        .await
    };

    activity.phase("finalizing").await;
    activity.counters(&outcome);

    let (status, error) = match &run_result {
        Ok(()) => ("success", None),
        Err(msg) => ("error", Some(msg.clone())),
    };

    let usage = json!({
        "run_id": agent_run_id.to_string(),
        "tool_calls": outcome.tool_calls,
        "elapsed_ms": start.elapsed().as_millis() as u64,
    });
    let agent_status = match &run_result {
        Ok(()) => "success",
        Err(m) if m.starts_with("tool_loop") || m.starts_with("timeout") => "budget_exceeded",
        Err(_) => "error",
    };
    if let Err(e) = persist_run(
        Some(&pool),
        agent_run_id,
        &prompt_user,
        &format!("{engine}/{model}"),
        tenant_uuid,
        Some(session_uuid),
        started_at,
        Utc::now(),
        agent_status,
        &usage,
        &Value::Array(trace),
    )
    .await
    {
        warn!(run_id = %run_id, error = %e, "failed to persist dreaming agent_runs row");
    }
    // Assistant summary turn — after persist_run so its run_id FK resolves.
    if !outcome.summary.is_empty() {
        sessions::persist_turn(
            Some(&pool),
            session_uuid,
            tenant_uuid,
            sctx.next_turn_index + 1,
            "assistant",
            &outcome.summary,
            Some(agent_run_id),
        )
        .await;
    }

    finalize_run_row(&pool, run_id, status, error.as_deref(), &outcome, activity.snapshot()).await;
    info!(
        run_id = %run_id,
        status,
        tool_calls = outcome.tool_calls,
        created = outcome.memories_created,
        merged = outcome.memories_merged,
        archived = outcome.memories_archived,
        "dreaming run finished"
    );

    match run_result {
        Ok(()) => Ok((run_id, outcome)),
        // Budget exhaustion is a normal-ish ending — the run row says success
        // when a summary landed, error otherwise; surface as Ok regardless so
        // the fabric job records the stats instead of retrying an LLM run.
        Err(msg) => {
            warn!(run_id = %run_id, error = %msg, "dreaming run ended abnormally");
            Ok((run_id, outcome))
        }
    }
}

/// Tally a tool call into the outcome + activity feed.
async fn observe_tool_call(
    outcome: &mut DreamingOutcome,
    activity: &mut Activity,
    tool: &str,
    args: &Value,
) {
    outcome.tool_calls += 1;
    let (icon, text) = match tool {
        "save_memory" | "save_memories" => {
            outcome.memories_created += 1;
            let title: String = args
                .get("title")
                .and_then(|v| v.as_str())
                .or_else(|| args.get("content").and_then(|v| v.as_str()))
                .unwrap_or("memory")
                .chars()
                .take(80)
                .collect();
            ("🧠", format!("Saving: {title}"))
        }
        "merge_memories" => {
            outcome.memories_merged += 1;
            ("⚖️", "Merging duplicate memories".to_string())
        }
        "update_memory_status" => {
            if args.get("status").and_then(|v| v.as_str()) == Some("archived") {
                outcome.memories_archived += 1;
                ("📦", "Archiving stale memory".to_string())
            } else {
                ("📦", "Updating memory status".to_string())
            }
        }
        "update_memory_importance" => {
            outcome.importance_rescored += 1;
            ("📈", "Re-scoring importance".to_string())
        }
        "memory_judge" => {
            outcome.judgements += 1;
            ("⚖️", "Judging memory conflict".to_string())
        }
        "link_memory_to_entity" | "ingest_entity" => {
            outcome.entities_linked += 1;
            ("🔗", "Linking memory ↔ entity".to_string())
        }
        "connector_read" => {
            outcome.connector_reads += 1;
            let op = args.get("operation").and_then(|v| v.as_str()).unwrap_or("read");
            ("🔌", format!("Connector read: {op}"))
        }
        "list_connectors" => ("🔌", "Listing connectors".to_string()),
        "memory_search" | "recall_memory" | "list_memories" => {
            let q: String = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(80)
                .collect();
            ("🔍", format!("Recalling: {q}"))
        }
        other => ("🔧", format!("Tool: {other}")),
    };
    activity.counters(outcome);
    activity.event(icon, text).await;
}

// ── adk engine path ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_via_adk(
    state: &AgentState,
    settings: &DreamingSettings,
    mode: &str,
    system_prompt: &str,
    user_prompt: &str,
    session_key: &str,
    wall_clock: Duration,
    activity: &mut Activity,
    outcome: &mut DreamingOutcome,
    trace: &mut Vec<Value>,
) -> Result<(), String> {
    use adk_rust::agent::LlmAgentBuilder;
    use adk_rust::runner::{Runner, RunnerConfig};
    use adk_rust::session::{CreateRequest, InMemorySessionService, SessionService};

    let cfg = state
        .engines
        .get()
        .await
        .map_err(|e| format!("engine: {e}"))?;
    let resolver =
        super::engine::CredentialResolver::new(state.credentials.clone(), state.tenant);
    let key = resolver.resolve(&cfg).await.map_err(|e| format!("creds: {e}"))?;
    let llm = super::engine::build_engine(&cfg, key).map_err(|e| format!("engine: {e}"))?;

    let shared = super::tools::SharedToolCtx {
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
        memory: state.memory.clone(),
    };
    let mutation_budget = Arc::new(MutationBudget::new(settings.mutation_cap));
    let read_budget = Arc::new(ConnectorReadBudget::new(
        settings.connector_read_budget,
        settings.connector_read_max_bytes,
    ));
    let connector_ctx = ConnectorToolCtx {
        pool: state.pool.clone(),
        credentials: state.credentials.clone(),
        tenant: state.tenant,
        budget: read_budget,
    };

    // Base toolset (same as the interactive agent), with the memory-mutating
    // tools wrapped in the run's mutation budget, plus the connector read
    // tools for gap-fill.
    let mut builder = LlmAgentBuilder::new(super::runner::AGENT_NAME)
        .description("Kyma dreaming agent — autonomous memory housekeeping.")
        .instruction(system_prompt)
        .model(llm);
    for tool in dreaming_toolset(&shared, connector_ctx, &mutation_budget, mode) {
        builder = builder.tool(tool);
    }
    let agent: Arc<dyn adk_rust::Agent> = Arc::new(
        builder
            .build()
            .map_err(|e| format!("agent build: {e:?}"))?,
    );

    let sessions_svc: Arc<dyn SessionService> = Arc::new(InMemorySessionService::new());
    sessions_svc
        .create(CreateRequest {
            app_name: super::runner::APP_NAME.to_string(),
            user_id: super::runner::ANON_USER.to_string(),
            session_id: Some(session_key.to_string()),
            state: Default::default(),
        })
        .await
        .map_err(|e| format!("session create: {e:?}"))?;
    let runner = Runner::new(RunnerConfig {
        app_name: super::runner::APP_NAME.to_string(),
        agent,
        session_service: sessions_svc,
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
    .map_err(|e| format!("runner build: {e:?}"))?;

    let user_id = UserId::new(super::runner::ANON_USER).map_err(|e| format!("user_id: {e}"))?;
    let session_id = SessionId::new(session_key).map_err(|e| format!("session_id: {e}"))?;
    let content = Content::new("user").with_text(user_prompt);
    let max_tool_calls = settings.max_tool_calls;

    let mut final_text = String::new();
    let run_future = async {
        let mut stream = runner
            .run(user_id, session_id, content)
            .await
            .map_err(|e| format!("runner.run: {e:?}"))?;
        while let Some(ev_result) = stream.next().await {
            let ev = ev_result.map_err(|e| format!("event: {e:?}"))?;
            let partial = ev.llm_response.partial;
            let parts: Vec<Part> = ev
                .llm_response
                .content
                .iter()
                .flat_map(|c| c.parts.iter().cloned())
                .collect();
            for part in parts {
                match part {
                    Part::Text { text } => {
                        if !partial {
                            final_text.push_str(&text);
                        }
                        trace.push(json!({"event": "answer_delta", "data": {"text": text}}));
                    }
                    Part::Thinking { thinking, .. } => {
                        activity.thinking(&thinking);
                        trace.push(json!({"event": "thinking_delta", "data": {"text": thinking}}));
                    }
                    Part::FunctionCall { name, args, .. } => {
                        trace.push(json!({"event": "tool_call", "data": {
                            "tool": name, "args": args, "call_index": outcome.tool_calls + 1
                        }}));
                        observe_tool_call(outcome, activity, &name, &args).await;
                        if outcome.tool_calls > max_tool_calls {
                            return Err(format!("tool_loop:{}", outcome.tool_calls));
                        }
                    }
                    Part::FunctionResponse {
                        function_response, ..
                    } => {
                        trace.push(json!({"event": "tool_result", "data": {
                            "tool": function_response.name,
                            "result": function_response.response,
                        }}));
                    }
                    _ => {}
                }
            }
        }
        Ok::<(), String>(())
    };

    let result = match tokio::time::timeout(wall_clock, run_future).await {
        Ok(r) => r,
        Err(_) => Err(format!("timeout:{}s", wall_clock.as_secs())),
    };
    outcome.summary = final_text;
    if let Err(msg) = &result {
        trace.push(json!({"event": "run_error", "data": {"code": "dreaming", "message": msg}}));
        activity.event("⚠️", format!("Run ended: {msg}")).await;
    }
    result
}

/// The dreaming toolset: every interactive tool, with mutating memory tools
/// wrapped in the run's [`MutationBudget`], plus the connector read tools.
fn dreaming_toolset(
    shared: &super::tools::SharedToolCtx,
    connector_ctx: ConnectorToolCtx,
    mutation_budget: &Arc<MutationBudget>,
    mode: &str,
) -> Vec<Arc<dyn Tool>> {
    use super::memory_tools::*;
    use super::tools::*;

    let mut tools: Vec<Arc<dyn Tool>> = vec![
        tool_list_databases(shared.clone()),
        tool_explore_schema(shared.clone()),
        tool_describe_table(shared.clone()),
        tool_run_kql(shared.clone()),
        tool_run_sql(shared.clone()),
        tool_sample_rows(shared.clone()),
        tool_find_references_to(shared.clone()),
        tool_graph_traverse(shared.clone()),
        tool_memory_search(shared.clone()),
        tool_recall_memory(shared.clone()),
        tool_list_memories(shared.clone()),
        tool_memory_compare(shared.clone()),
        tool_flush_memory(shared.clone()),
    ];
    for mutating in [
        tool_save_memory(shared.clone()),
        tool_save_memories(shared.clone()),
        tool_link_memory_to_entity(shared.clone()),
        tool_ingest_entity(shared.clone()),
        tool_update_memory_status(shared.clone()),
        tool_update_memory_importance(shared.clone()),
        tool_merge_memories(shared.clone()),
        tool_memory_judge(shared.clone()),
    ] {
        tools.push(Arc::new(BudgetedTool {
            inner: mutating,
            budget: mutation_budget.clone(),
        }));
    }
    // Gap-fill tools only when the mode allows them (the prompt matches).
    if mode != "housekeeping_only" {
        tools.push(tool_list_connectors(connector_ctx.clone()));
        tools.push(tool_connector_read(connector_ctx));
    }
    tools
}

// ── Claude CLI engine path ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_via_claude_cli(
    state: &AgentState,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    wall_clock: Duration,
    activity: &mut Activity,
    outcome: &mut DreamingOutcome,
    trace: &mut Vec<Value>,
) -> Result<(), String> {
    // Tools come via kyma's own MCP endpoint. Headless runs can carry an
    // internal bearer (KYMA_INTERNAL_BEARER) for auth-enabled deployments;
    // auth-disabled dev mode needs none.
    let auth_header = std::env::var("KYMA_INTERNAL_BEARER")
        .ok()
        .map(|t| format!("Bearer {t}"));
    let mcp = state.mcp_url.clone().map(|url| claude_cli::McpConfig {
        url,
        auth_header,
        // Headless background run: ONLY kyma's MCP server — the user's own
        // MCP servers/plugins must not be reachable from a dreaming agent.
        strict: true,
    });

    // Scratch working directory: keeps repo-local CLAUDE.md / .claude
    // settings out of the headless agent's context (it must see the memory
    // store through kyma's tools, not the operator's checkout).
    let scratch = std::env::temp_dir().join("kyma-dreaming");
    let _ = std::fs::create_dir_all(&scratch);

    // The CLI takes one prompt: fold the dreaming instructions + task in.
    let prompt = format!("{system_prompt}\n\n---\n\n{user_prompt}");
    let (mut events, pid) = claude_cli::run_stream_with_pid(
        &prompt,
        Some(model),
        None,
        Some(&scratch),
        mcp.as_ref(),
    )
    .map_err(|e| format!("claude spawn: {e}"))?;

    let mut answer = String::new();
    let deadline = tokio::time::Instant::now() + wall_clock;
    let mut errored: Option<String> = None;
    loop {
        let ev = match tokio::time::timeout_at(deadline, events.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => break, // stream closed
            Err(_) => {
                // Wall clock exceeded — kill the runaway agent.
                if let Some(pid) = pid {
                    let _ = std::process::Command::new("kill")
                        .arg(pid.to_string())
                        .status();
                }
                errored = Some(format!("timeout:{}s", wall_clock.as_secs()));
                break;
            }
        };
        match ev {
            claude_cli::ClaudeEvent::Init { session_id } => {
                trace.push(json!({"event": "session", "data": {"session_id": session_id}}));
            }
            claude_cli::ClaudeEvent::TextDelta { text, .. } => {
                answer.push_str(&text);
                trace.push(json!({"event": "answer_delta", "data": {"text": text}}));
            }
            claude_cli::ClaudeEvent::ThinkingDelta { text, .. } => {
                activity.thinking(&text);
                trace.push(json!({"event": "thinking_delta", "data": {"text": text}}));
            }
            claude_cli::ClaudeEvent::ToolUse { name, input, .. } => {
                trace.push(json!({"event": "tool_call", "data": {
                    "tool": name, "args": input, "call_index": outcome.tool_calls + 1
                }}));
                // MCP tools arrive as `mcp__kyma__save_memory` — strip the prefix
                // so tallies and the activity feed read naturally.
                let short = name.rsplit("__").next().unwrap_or(&name).to_string();
                observe_tool_call(outcome, activity, &short, &input).await;
            }
            claude_cli::ClaudeEvent::ToolResult { output, is_error, .. } => {
                trace.push(json!({"event": "tool_result", "data": {
                    "tool": "", "result": output, "is_error": is_error
                }}));
            }
            claude_cli::ClaudeEvent::Result { is_error, .. } => {
                if is_error {
                    errored.get_or_insert_with(|| "claude reported an error".to_string());
                }
            }
            claude_cli::ClaudeEvent::Error { message } => {
                trace.push(json!({"event": "run_error", "data": {"message": message}}));
                errored = Some(message);
            }
            _ => {}
        }
    }
    outcome.summary = answer;
    match errored {
        None => Ok(()),
        Some(msg) => {
            activity.event("⚠️", format!("Run ended: {msg}")).await;
            Err(msg)
        }
    }
}

// ── scheduler ────────────────────────────────────────────────────────────────

/// Enqueues a `dreaming` fabric job on an interval when dreaming is enabled.
/// Execution is the worker's business — this loop only schedules.
pub struct DreamingScheduler {
    state: AgentState,
    fabric: Arc<kyma_catalog::PgFabricStore>,
    pub poll: Duration,
}

impl DreamingScheduler {
    pub fn new(state: AgentState, fabric: Arc<kyma_catalog::PgFabricStore>) -> Self {
        Self {
            state,
            fabric,
            poll: Duration::from_secs(60),
        }
    }

    pub async fn tick_once(&self) -> anyhow::Result<()> {
        let Some(pool) = self.state.pool.as_ref() else {
            return Ok(());
        };
        let settings = memory_settings::load(Some(pool), self.state.tenant).await.dreaming;
        if !settings.enabled {
            return Ok(());
        }
        // Due when no dreaming run started within the interval.
        let last: Option<(chrono::DateTime<Utc>,)> = sqlx::query_as(
            "SELECT started_at FROM memory_pipeline_runs \
             WHERE tenant_id = $1 AND kind = 'dreaming' \
             ORDER BY started_at DESC LIMIT 1",
        )
        .bind(self.state.tenant.as_uuid())
        .fetch_optional(pool)
        .await?;
        if let Some((started,)) = last {
            let elapsed = Utc::now().signed_duration_since(started);
            if elapsed.num_seconds() < settings.interval_secs as i64 {
                return Ok(());
            }
        }
        // jobs_dreaming_uniq makes this a no-op while one is already in flight.
        let enqueued = self
            .fabric
            .enqueue_job(
                self.state.tenant,
                &kyma_core::fabric::EnqueueJob {
                    kind: kyma_core::fabric::JOB_DREAMING.to_string(),
                    payload: serde_json::to_value(DreamingRequest {
                        trigger: Trigger::Scheduled,
                        mode: None,
                        focus: None,
                        job_id: None,
                        worker_id: None,
                    })?,
                    priority: 0,
                    affinity_worker_id: None,
                    req_capabilities: vec!["dreaming".into()],
                    label_selector: json!({}),
                    max_attempts: 1,
                },
            )
            .await?;
        if let Some(job_id) = enqueued {
            info!(job_id = %job_id, "dreaming job scheduled");
        }
        Ok(())
    }

    pub async fn run(self, shutdown: impl std::future::Future<Output = ()>) {
        info!("dreaming scheduler starting (runs only when enabled in memory settings)");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("dreaming scheduler shutdown"); return; }
                _ = tokio::time::sleep(self.poll) => {
                    if let Err(e) = self.tick_once().await {
                        warn!(error = %e, "dreaming scheduler tick failed");
                    }
                }
            }
        }
    }
}

// ── HTTP handlers (mounted under /v1/agent/memory/dreaming) ──────────────────

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

#[derive(Deserialize)]
pub struct RunsQuery {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default = "default_runs_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_runs_limit() -> i64 {
    25
}

const RUN_SELECT: &str = "SELECT id, kind, status, mode, trigger, engine, model, worker_id, \
        started_at, finished_at, events_scanned, memories_written, error, \
        job_id, session_id, agent_run_id, stats_json, progress_json \
 FROM memory_pipeline_runs";

fn run_row_json(r: &sqlx::postgres::PgRow) -> Value {
    use sqlx::Row as _;
    json!({
        "id": r.get::<Uuid, _>("id").to_string(),
        "kind": r.get::<String, _>("kind"),
        "status": r.get::<String, _>("status"),
        "mode": r.get::<String, _>("mode"),
        "trigger": r.get::<String, _>("trigger"),
        "engine": r.get::<Option<String>, _>("engine"),
        "model": r.get::<Option<String>, _>("model"),
        "worker_id": r.get::<Option<Uuid>, _>("worker_id").map(|u| u.to_string()),
        "started_at": r.get::<chrono::DateTime<Utc>, _>("started_at").to_rfc3339(),
        "finished_at": r.get::<Option<chrono::DateTime<Utc>>, _>("finished_at").map(|t| t.to_rfc3339()),
        "events_scanned": r.get::<i64, _>("events_scanned"),
        "memories_written": r.get::<i64, _>("memories_written"),
        "error": r.get::<Option<String>, _>("error"),
        "job_id": r.get::<Option<Uuid>, _>("job_id").map(|u| u.to_string()),
        "session_id": r.get::<Option<Uuid>, _>("session_id").map(|u| u.to_string()),
        "agent_run_id": r.get::<Option<Uuid>, _>("agent_run_id").map(|u| u.to_string()),
        "stats": r.get::<Option<Value>, _>("stats_json"),
        "progress": r.get::<Option<Value>, _>("progress_json"),
    })
}

/// GET /memory/dreaming/runs — unified runs feed (dreaming + consolidation).
pub async fn list_runs_handler(
    State(state): State<AgentState>,
    Query(q): Query<RunsQuery>,
) -> impl IntoResponse {
    let Some(pool) = state.pool.as_ref() else {
        return (StatusCode::OK, Json(json!({ "items": [] }))).into_response();
    };
    let rows = sqlx::query(&format!(
        "{RUN_SELECT} WHERE tenant_id = $1 AND ($2::text IS NULL OR kind = $2) \
         ORDER BY started_at DESC LIMIT $3 OFFSET $4"
    ))
    .bind(state.tenant.as_uuid())
    .bind(&q.kind)
    .bind(q.limit.clamp(1, 200))
    .bind(q.offset.max(0))
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => {
            let items: Vec<Value> = rows.iter().map(run_row_json).collect();
            (StatusCode::OK, Json(json!({ "items": items }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /memory/dreaming/runs/:id — one run with stats + linkage. For an
/// in-flight run, the live activity feed is read off the fabric job.
pub async fn get_run_handler(
    State(state): State<AgentState>,
    AxumPath(id): AxumPath<Uuid>,
) -> impl IntoResponse {
    let Some(pool) = state.pool.as_ref() else {
        return (StatusCode::NOT_FOUND, Json(json!({"error": "no runs in local mode"})))
            .into_response();
    };
    let row = sqlx::query(&format!("{RUN_SELECT} WHERE tenant_id = $1 AND id = $2"))
        .bind(state.tenant.as_uuid())
        .bind(id)
        .fetch_optional(pool)
        .await;
    match row {
        Ok(Some(r)) => {
            let mut body = run_row_json(&r);
            // Live progress for a running run comes from the fabric job row.
            use sqlx::Row as _;
            if body.get("status").and_then(|s| s.as_str()) == Some("running") {
                if let Some(job_id) = r.get::<Option<Uuid>, _>("job_id") {
                    if let Ok(Some(p)) = sqlx::query_scalar::<_, Value>(
                        "SELECT progress FROM jobs WHERE id = $1",
                    )
                    .bind(job_id)
                    .fetch_optional(pool)
                    .await
                    {
                        body["progress"] = p;
                    }
                }
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"error": "no such run"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[derive(Deserialize, Default)]
pub struct TriggerBody {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub focus: Option<String>,
}

/// POST /memory/dreaming/run — manual trigger: enqueue a dreaming job now.
/// Deduped while one is already in flight (`jobs_dreaming_uniq`).
pub async fn trigger_run_handler(
    State(state): State<AgentState>,
    body: Option<Json<TriggerBody>>,
) -> impl IntoResponse {
    let Some(pool) = state.pool.clone() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "dreaming requires Postgres (local mode)"})),
        )
            .into_response();
    };
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let fabric = kyma_catalog::PgFabricStore::new(pool);
    let payload = match serde_json::to_value(DreamingRequest {
        trigger: Trigger::Manual,
        mode: body.mode,
        focus: body.focus,
        job_id: None,
        worker_id: None,
    }) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response()
        }
    };
    match fabric
        .enqueue_job(
            state.tenant,
            &kyma_core::fabric::EnqueueJob {
                kind: kyma_core::fabric::JOB_DREAMING.to_string(),
                payload,
                priority: 10, // manual runs jump the queue
                affinity_worker_id: None,
                req_capabilities: vec!["dreaming".into()],
                label_selector: json!({}),
                max_attempts: 1,
            },
        )
        .await
    {
        Ok(Some(job_id)) => (
            StatusCode::ACCEPTED,
            Json(json!({ "job_id": job_id })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::OK,
            Json(json!({ "job_id": Value::Null, "deduped": true,
                          "detail": "a dreaming run is already in flight" })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
