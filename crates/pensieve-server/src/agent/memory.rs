//! Memory ingestion visibility + the scheduled consolidation ("dreaming")
//! pipeline that backs the Agent-tab Memory panel.
//!
//! - [`overview_handler`] (`GET /v1/agent/memory/overview`) aggregates three
//!   things for the UI: the memory store (`memory.memory_nodes`), the live
//!   conversation firehose (`default.claude_code_events`), and recent pipeline
//!   runs (`memory_pipeline_runs`).
//! - [`MemoryConsolidator`] is a background loop that periodically distills NEW
//!   firehose activity into durable `summary` memories and records each run.
//!   It is deterministic (needs no LLM); when an agent engine is configured the
//!   distillation can be upgraded to a full agent run — the run-recording and
//!   visibility surface is identical either way.

use std::time::Duration;

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use kyma_core::tenant::TenantId;
use kyma_memory::types::MemoryType;
use kyma_memory::{CreateMemory, MemoryWriter};

use super::engine::EngineKind;
use super::memory_conflict::{self, ConflictTally};
use super::memory_gate::HitlGate;
use super::memory_queue_store::QueueStore;
use super::memory_settings::ValidityGateSettings;
use super::state::AgentState;
use super::tools::{execute_sql, SharedToolCtx};
use super::{memory_extract, memory_resolve, memory_settings, memory_validity_gate};

const FIREHOSE_DB: &str = "default";
const MEMORY_DB: &str = kyma_memory::DEFAULT_DATABASE; // "memory"

// ── helpers ──────────────────────────────────────────────────────────────────

fn rows_of(v: &Value) -> Vec<Value> {
    v.get("rows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Escape a string for safe inlining inside a single-quoted SQL literal.
/// Shared by the agent SQL-building call sites (one copy so the escaping rule
/// can be hardened in a single place).
pub(crate) fn sql_lit(s: &str) -> String {
    s.replace('\'', "''")
}

async fn build_writer(shared: &SharedToolCtx) -> anyhow::Result<MemoryWriter> {
    let embed = kyma_memory::shared_embedding()
        .await
        .map_err(|e| anyhow::anyhow!("embedding backend: {e}"))?;
    Ok(MemoryWriter::new(
        shared.catalog.clone(),
        shared.format.clone(),
        embed,
    ))
}

fn shared_from(state: &AgentState) -> SharedToolCtx {
    SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: Some(kyma_federation::runtime_from(state.credentials.clone())),
        catalog: state.catalog.clone(),
        format: state.format.clone(),
        pool: state.pool.clone(),
        memory: state.memory.clone(),
        hitl: None,
        memory_settings_path: state.memory_settings_path.clone(),
    }
}

// ── GET /v1/agent/memory/overview ────────────────────────────────────────────

pub async fn overview_handler(State(state): State<AgentState>) -> Json<Value> {
    let shared = shared_from(&state);
    Json(json!({
        "memory": memory_section(&shared).await,
        "firehose": firehose_section(&shared).await,
        "pipeline_runs": runs_section(state.pool.as_ref(), state.tenant).await,
    }))
}

async fn memory_section(shared: &SharedToolCtx) -> Value {
    // Dedup to the latest version per id (memory is append-only / latest-wins).
    let counts_sql = "WITH latest AS (SELECT memory_type, status, realm, \
        row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM memory_nodes) \
        SELECT memory_type, status, realm, count(*) AS n FROM latest WHERE rn = 1 \
        GROUP BY memory_type, status, realm";
    let recent_sql = "WITH latest AS (SELECT id, memory_type, realm, status, importance, \
        content_preview, provenance, created_at, \
        row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM memory_nodes) \
        SELECT id, memory_type, realm, status, importance, content_preview, provenance, created_at \
        FROM latest WHERE rn = 1 ORDER BY created_at DESC LIMIT 12";
    json!({
        "counts": rows_of(&execute_sql(shared, MEMORY_DB, counts_sql, 1000).await),
        "recent": rows_of(&execute_sql(shared, MEMORY_DB, recent_sql, 12).await),
    })
}

// ── GET /v1/agent/memory/source-summary ──────────────────────────────────────

/// Memories grouped by provenance source + realm (latest version per id,
/// archived excluded; no provenance source = "manual"). Powers the Data
/// Sources "Memories" tab. The engine has no JSON UDFs, so the SQL groups by
/// the raw provenance blob and `provenance.source` is folded out in Rust.
pub async fn source_summary_handler(State(state): State<AgentState>) -> Json<Value> {
    let shared = shared_from(&state);
    let sql = kyma_memory::sql::source_summary_sql(kyma_memory::NODE_TABLE);
    // An empty/unprovisioned memory store surfaces as an execute_sql error;
    // rows_of maps that to no rows, i.e. an empty summary.
    let rows = rows_of(&execute_sql(&shared, MEMORY_DB, &sql, 100_000).await);
    let items = kyma_memory::sql::fold_source_summary(&rows);
    Json(json!({ "items": items }))
}

async fn firehose_section(shared: &SharedToolCtx) -> Value {
    let by_kind =
        "SELECT kind, count(*) AS n FROM claude_code_events GROUP BY kind ORDER BY n DESC";
    let timeline =
        "SELECT date_bin(INTERVAL '1 hour', ts, TIMESTAMP '1970-01-01 00:00:00') AS bucket, \
        count(*) AS n FROM claude_code_events GROUP BY bucket ORDER BY bucket";
    let sessions = "SELECT session_id, max(realm) AS realm, count(*) AS events, \
        min(ts) AS first_seen, max(ts) AS last_seen FROM claude_code_events \
        GROUP BY session_id ORDER BY last_seen DESC LIMIT 20";
    let recent = "SELECT ts, kind, session_id, realm, tool_name, substr(text, 1, 200) AS text \
        FROM claude_code_events ORDER BY ts DESC LIMIT 30";
    json!({
        "by_kind": rows_of(&execute_sql(shared, FIREHOSE_DB, by_kind, 100).await),
        "timeline": rows_of(&execute_sql(shared, FIREHOSE_DB, timeline, 1000).await),
        "sessions": rows_of(&execute_sql(shared, FIREHOSE_DB, sessions, 20).await),
        "recent": rows_of(&execute_sql(shared, FIREHOSE_DB, recent, 30).await),
    })
}

#[allow(clippy::type_complexity)]
async fn runs_section(pool: Option<&PgPool>, tenant: TenantId) -> Value {
    let Some(pool) = pool else {
        return Value::Array(vec![]);
    }; // local: no pipeline runs
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            String,
            DateTime<Utc>,
            Option<DateTime<Utc>>,
            i64,
            i64,
            Option<String>,
        ),
    >(
        "SELECT id, kind, status, started_at, finished_at, events_scanned, memories_written, error \
         FROM memory_pipeline_runs WHERE tenant_id = $1 ORDER BY started_at DESC LIMIT 20",
    )
    .bind(tenant.as_uuid())
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let arr: Vec<Value> = rows
        .into_iter()
        .map(
            |(id, kind, status, started, finished, scanned, written, error)| {
                json!({
                    "id": id.to_string(),
                    "kind": kind,
                    "status": status,
                    "started_at": started.to_rfc3339(),
                    "finished_at": finished.map(|t| t.to_rfc3339()),
                    "events_scanned": scanned,
                    "memories_written": written,
                    "error": error,
                })
            },
        )
        .collect();
    Value::Array(arr)
}

// ── background consolidation pipeline ────────────────────────────────────────

/// Aggregated results of one consolidation tick (summed across realms).
#[derive(Debug, Default)]
struct ConsolidateOutcome {
    scanned: i64,
    written: i64,
    entities: i64,
    relationships: i64,
    tally: ConflictTally,
    mode: String,
}

impl ConsolidateOutcome {
    /// Fold a per-realm partial result into the running total.
    fn absorb(&mut self, other: ConsolidateOutcome) {
        self.scanned += other.scanned;
        self.written += other.written;
        self.entities += other.entities;
        self.relationships += other.relationships;
        self.tally.merge(&other.tally);
    }

    /// The A.U.D.N. decision breakdown as JSON, or `None` for deterministic runs.
    fn decisions_json(&self) -> Option<Value> {
        if self.mode == "extraction" {
            Some(self.tally.to_json())
        } else {
            None
        }
    }
}

/// Periodically distills new conversation-firehose activity into durable
/// memories, recording each tick in `memory_pipeline_runs`. When an agent
/// engine is configured ([`MemoryConsolidator::with_engine`]) it runs LLM
/// extraction + conflict resolution; otherwise it writes deterministic
/// activity summaries.
pub struct MemoryConsolidator {
    shared: SharedToolCtx,
    pool: PgPool,
    tenant: TenantId,
    pub poll_interval: Duration,
    /// When set (and the configured engine isn't `claude_cli`), each tick runs
    /// LLM extraction + conflict resolution instead of deterministic summaries.
    engine: Option<AgentState>,
}

impl MemoryConsolidator {
    pub fn new(shared: SharedToolCtx, pool: PgPool, tenant: TenantId) -> Self {
        Self {
            shared,
            pool,
            tenant,
            poll_interval: Duration::from_secs(60),
            engine: None,
        }
    }

    /// Enable intelligent (LLM) extraction by supplying the agent state whose
    /// configured engine drives extraction + conflict-resolution turns. Without
    /// it (or with a `claude_cli` engine) the pipeline stays deterministic.
    pub fn with_engine(mut self, state: AgentState) -> Self {
        self.engine = Some(state);
        self
    }

    pub async fn run(self, shutdown: impl std::future::Future<Output = ()> + Send) {
        let mut ticker = tokio::time::interval(self.poll_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // consume the immediate first tick
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => break,
                _ = ticker.tick() => {
                    if let Err(e) = self.tick().await {
                        tracing::warn!(error = %e, "memory consolidation tick failed");
                    }
                }
            }
        }
    }

    async fn tick(&self) -> anyhow::Result<()> {
        let now = Utc::now();
        let last_wm: Option<DateTime<Utc>> = sqlx::query_scalar::<_, Option<DateTime<Utc>>>(
            "SELECT max(watermark_ts) FROM memory_pipeline_runs \
             WHERE tenant_id = $1 AND status = 'success'",
        )
        .bind(self.tenant.as_uuid())
        .fetch_one(&self.pool)
        .await
        .ok()
        .flatten();

        let ts_filter = match last_wm {
            Some(wm) => format!(
                "WHERE ts > CAST('{}' AS TIMESTAMP)",
                wm.format("%Y-%m-%dT%H:%M:%S%.6f")
            ),
            None => String::new(),
        };

        // Which realms have new firehose activity since the watermark?
        let realms_sql = format!(
            "SELECT realm, count(*) AS events, count(DISTINCT session_id) AS sessions \
             FROM claude_code_events {ts_filter} GROUP BY realm"
        );
        let res = execute_sql(&self.shared, FIREHOSE_DB, &realms_sql, 1000).await;
        if res.get("error").is_some() {
            // Firehose table not created yet — nothing to consolidate. Stay quiet.
            return Ok(());
        }
        let realms = rows_of(&res);
        if realms.is_empty() {
            return Ok(()); // no new activity this tick
        }

        let run_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO memory_pipeline_runs (id, tenant_id, kind, status, started_at) \
             VALUES ($1, $2, 'consolidation', 'running', $3)",
        )
        .bind(run_id)
        .bind(self.tenant.as_uuid())
        .bind(now)
        .execute(&self.pool)
        .await?;

        match self.consolidate(&realms, &ts_filter).await {
            Ok(o) => {
                sqlx::query(
                    "UPDATE memory_pipeline_runs SET status='success', finished_at=$2, \
                     events_scanned=$3, memories_written=$4, watermark_ts=$5, \
                     entities_written=$6, relationships_written=$7, decisions_json=$8, \
                     mode=$9 WHERE id=$1",
                )
                .bind(run_id)
                .bind(Utc::now())
                .bind(o.scanned)
                .bind(o.written)
                .bind(now)
                .bind(o.entities)
                .bind(o.relationships)
                .bind(o.decisions_json())
                .bind(o.mode)
                .execute(&self.pool)
                .await?;
            }
            Err(e) => {
                sqlx::query(
                    "UPDATE memory_pipeline_runs SET status='error', finished_at=$2, error=$3 \
                     WHERE id=$1",
                )
                .bind(run_id)
                .bind(Utc::now())
                .bind(e.to_string())
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    async fn consolidate(
        &self,
        realms: &[Value],
        ts_filter: &str,
    ) -> anyhow::Result<ConsolidateOutcome> {
        let writer = build_writer(&self.shared).await?;
        let _ = writer.ensure_provisioned().await;
        let settings = memory_settings::load(Some(&self.pool), self.tenant).await;
        // HITL chokepoint — built once per tick. Only when the policy is on;
        // otherwise consolidation applies directly (zero overhead, no rows).
        let gate = settings.hitl.enabled.then(|| HitlGate {
            policy: settings.hitl.clone(),
            store: std::sync::Arc::new(QueueStore::Pg {
                pool: self.pool.clone(),
                tenant: self.tenant,
            }),
            resolver: None,
            source: "realtime",
            source_run_id: None,
        });
        // LLM extraction only when the user enabled it AND a usable engine exists.
        let engine = if settings.extraction_enabled {
            self.usable_engine().await
        } else {
            None
        };
        let mut out = ConsolidateOutcome {
            mode: if engine.is_some() {
                "extraction"
            } else {
                "deterministic"
            }
            .to_string(),
            ..Default::default()
        };
        for r in realms {
            let realm = r.get("realm").and_then(Value::as_str).unwrap_or("default");
            let events = r.get("events").and_then(Value::as_i64).unwrap_or(0);
            let sessions = r.get("sessions").and_then(Value::as_i64).unwrap_or(0);
            out.scanned += events;
            // Honor the min-events gate before doing any (LLM or summary) work.
            if events < settings.min_events {
                continue;
            }
            if let Some(state) = engine {
                match self
                    .extract_realm(
                        state,
                        &writer,
                        realm,
                        ts_filter,
                        gate.as_ref(),
                        &settings.validity_gate,
                    )
                    .await
                {
                    Ok(partial) => {
                        out.absorb(partial);
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(realm, error = %e, "memory extraction failed; deterministic fallback");
                    }
                }
            }
            out.written += self
                .deterministic_summary(&writer, realm, events, sessions, ts_filter)
                .await;
        }
        Ok(out)
    }

    /// The configured engine usable for adk extraction — `Some` only when one
    /// is present and isn't `claude_cli` (which can't run through adk-rust).
    async fn usable_engine(&self) -> Option<&AgentState> {
        let state = self.engine.as_ref()?;
        match state.engines.get().await {
            Ok(cfg) if cfg.kind != EngineKind::ClaudeCli => Some(state),
            _ => None,
        }
    }

    /// LLM extraction → entity resolution/linking → conflict resolution for one
    /// realm's new activity. Errors bubble so the caller can fall back.
    async fn extract_realm(
        &self,
        state: &AgentState,
        writer: &MemoryWriter,
        realm: &str,
        ts_filter: &str,
        gate: Option<&HitlGate>,
        validity_gate: &ValidityGateSettings,
    ) -> anyhow::Result<ConsolidateOutcome> {
        let window = self.fetch_window(realm, ts_filter).await;
        let mut out = ConsolidateOutcome::default();
        if window.trim().is_empty() {
            return Ok(out);
        }
        let bundle = memory_extract::extract(state, realm, None, &window).await?;
        if bundle.is_empty() {
            return Ok(out);
        }

        // M8.2: capture the raw window as an Activity before writing any
        // memories, so each one can link back to it (worked-example /
        // precedent retrieval). Best-effort — extraction still proceeds
        // without a precedent link if capture fails.
        let activity_id = self.capture_activity(realm, &window).await;

        // M8.3b: drop trivial/filler candidates before they're even
        // considered for entity linking or conflict resolution. Off by
        // default — a disabled gate never rejects anything.
        let mut rejected_trivial = 0i64;
        let memories: Vec<_> = bundle
            .memories
            .into_iter()
            .filter(
                |m| match memory_validity_gate::extraction_reject(m, validity_gate) {
                    Some(_) => {
                        rejected_trivial += 1;
                        false
                    }
                    None => true,
                },
            )
            .collect();

        let resolved = memory_resolve::resolve_and_link(
            &self.shared,
            writer,
            realm,
            &bundle.entities,
            &bundle.relationships,
        )
        .await;
        out.entities += resolved.entities_written;
        out.relationships += resolved.relationships_written;

        for m in &memories {
            let refs: Vec<String> = m
                .entity_mentions
                .iter()
                .filter_map(|name| {
                    resolved
                        .entity_nodes
                        .get(&name.trim().to_ascii_lowercase())
                        .cloned()
                })
                .collect();
            let provenance = json!({ "source": "claude-code", "realm": realm });
            let t = memory_conflict::consolidate_memory(
                state,
                &self.shared,
                writer,
                realm,
                m,
                refs,
                provenance,
                gate,
                activity_id.as_deref(),
            )
            .await;
            out.tally.merge(&t);
        }
        out.tally.rejected_trivial += rejected_trivial;
        out.written += out.tally.written();
        Ok(out)
    }

    /// Capture `window` as an Activity (M8.2) in the isolated `activities`
    /// database. Returns its node id, or `None` on any failure (embedding
    /// backend unavailable, provisioning error, …) — callers proceed without
    /// a precedent link rather than failing extraction over this.
    async fn capture_activity(&self, realm: &str, window: &str) -> Option<String> {
        let embed = kyma_memory::shared_embedding().await.ok()?;
        let awriter = MemoryWriter::new(
            self.shared.catalog.clone(),
            self.shared.format.clone(),
            embed,
        )
        .with_database(kyma_memory::activities::ACTIVITIES_DB);
        let activity = kyma_memory::activities::RawActivity {
            text: window.to_string(),
            realm: realm.to_string(),
            source: "claude-code".to_string(),
        };
        match kyma_memory::activities::capture(&awriter, &activity).await {
            Ok(id) => Some(kyma_memory::activities::activity_node_id(&id)),
            Err(e) => {
                tracing::debug!(error = %e, "activity capture failed");
                None
            }
        }
    }

    /// Build a compact transcript of a realm's new firehose activity (oldest
    /// first) to feed the extractor.
    async fn fetch_window(&self, realm: &str, ts_filter: &str) -> String {
        let realm_lit = sql_lit(realm);
        let clause = if ts_filter.is_empty() {
            format!("WHERE realm = '{realm_lit}'")
        } else {
            format!("{ts_filter} AND realm = '{realm_lit}'")
        };
        let sql = format!(
            "SELECT ts, kind, tool_name, substr(text, 1, 400) AS text \
             FROM claude_code_events {clause} ORDER BY ts ASC LIMIT 60"
        );
        let rows = rows_of(&execute_sql(&self.shared, FIREHOSE_DB, &sql, 60).await);
        let mut out = String::new();
        for r in &rows {
            let kind = r.get("kind").and_then(Value::as_str).unwrap_or("event");
            let text = r.get("text").and_then(Value::as_str).unwrap_or("");
            let tool = r.get("tool_name").and_then(Value::as_str).unwrap_or("");
            if !text.is_empty() {
                out.push_str(&format!("[{kind}] {text}\n"));
            } else if !tool.is_empty() {
                out.push_str(&format!("[{kind}] tool: {tool}\n"));
            }
        }
        out
    }

    /// Deterministic fallback: write one activity-summary memory for a realm.
    /// Returns the number of memories written (0 or 1).
    async fn deterministic_summary(
        &self,
        writer: &MemoryWriter,
        realm: &str,
        events: i64,
        sessions: i64,
        ts_filter: &str,
    ) -> i64 {
        let detail = self.realm_detail(realm, ts_filter).await;
        let content = format!(
            "Claude Code activity in project \"{realm}\": {events} new event(s) across \
             {sessions} session(s). {detail}"
        );
        let mut cm = CreateMemory::new(content);
        cm.title = Some(format!("Session activity — {realm}"));
        cm.memory_type = MemoryType::Summary;
        cm.realm = realm.to_string();
        cm.importance = 0.4;
        cm.tags = vec![
            "pipeline:consolidation".to_string(),
            "source:claude-code".to_string(),
        ];
        if writer.save(&cm).await.is_ok() {
            1
        } else {
            0
        }
    }

    /// Build a short deterministic summary detail (tools used + recent prompts)
    /// for one realm's new activity.
    async fn realm_detail(&self, realm: &str, ts_filter: &str) -> String {
        let realm_lit = sql_lit(realm);
        let realm_clause = if ts_filter.is_empty() {
            format!("WHERE realm = '{realm_lit}'")
        } else {
            format!("{ts_filter} AND realm = '{realm_lit}'")
        };
        let tools_sql = format!(
            "SELECT DISTINCT tool_name FROM claude_code_events {realm_clause} \
             AND tool_name IS NOT NULL AND tool_name <> '' LIMIT 12"
        );
        let tools: Vec<String> =
            rows_of(&execute_sql(&self.shared, FIREHOSE_DB, &tools_sql, 12).await)
                .iter()
                .filter_map(|r| r.get("tool_name").and_then(Value::as_str).map(String::from))
                .collect();
        let prompts_sql = format!(
            "SELECT text FROM claude_code_events {realm_clause} AND kind = 'user_prompt' \
             AND text IS NOT NULL AND text <> '' ORDER BY ts DESC LIMIT 3"
        );
        let prompts: Vec<String> =
            rows_of(&execute_sql(&self.shared, FIREHOSE_DB, &prompts_sql, 3).await)
                .iter()
                .filter_map(|r| {
                    r.get("text")
                        .and_then(Value::as_str)
                        .map(|s| s.chars().take(120).collect::<String>())
                })
                .collect();
        let mut out = String::new();
        if !tools.is_empty() {
            out.push_str(&format!("Tools used: {}. ", tools.join(", ")));
        }
        if !prompts.is_empty() {
            out.push_str(&format!("Recent requests: {}.", prompts.join(" | ")));
        }
        out
    }
}
