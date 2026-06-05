//! Curation decision engine for Claude Code memory files.
//!
//! Decides — from the memory DB alone — what kyma should do to a project's
//! `~/.claude/projects/<slug>/memory/` directory: which high-value memories
//! to **promote** into native `.md` files Claude Code loads, which files to
//! **archive** (superseded, duplicated, demoted — never deleted), and what
//! the kyma-managed region of `MEMORY.md` should contain.
//!
//! This module performs **no filesystem IO**. It emits a serializable plan
//! of [`FileAction`]s (the audit/dry-run record) and applies the matching DB
//! mutations in the same pass, so store and files converge; the applier in
//! `kyma-local` owns the actual writes (atomic, lock-guarded, user-edit
//! aware).
//!
//! Anti-flood invariants: a hard cap on managed index entries
//! ([`CurationConfig::promote_max`]), promotion **hysteresis** (a promoted
//! memory keeps its slot unless it falls below `0.8 × min_importance` or is
//! invalidated), and `summary`/`entity` memories never promote.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use kyma_memory::MemoryWriter;

use super::SharedToolCtx;

/// One entry of the managed `MEMORY.md` region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub title: String,
    pub file: String,
    pub hook: String,
}

/// A single file-level action the applier should take. Serializable: the
/// plan doubles as the dry-run output and the audit-log record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum FileAction {
    /// Write (create or refresh) a kyma-promoted memory file.
    WriteMemoryFile {
        /// Filename relative to the project's `memory/` dir.
        file: String,
        /// Full rendered file (frontmatter + body).
        content: String,
        node_id: String,
        /// Normalized hash of the rendered body — the user-edit guard.
        content_hash: String,
    },
    /// Move a file into `memory/archive/` with a tombstone (never delete).
    ArchiveFile {
        file: String,
        reason: String,
        node_id: Option<String>,
    },
    /// Replace the kyma-managed region of `MEMORY.md` with these entries.
    SetIndex { entries: Vec<IndexEntry> },
}

/// Counters for one curation pass.
#[derive(Debug, Default, Clone, Serialize)]
pub struct CurationOutcome {
    /// Memories newly promoted to files this pass.
    pub promoted: usize,
    /// Promoted files re-rendered because their memory changed.
    pub refreshed: usize,
    /// Files archived (superseded, duplicate, demoted).
    pub archived_files: usize,
    /// Duplicate nodes merged in the DB.
    pub merged: usize,
    /// Entries in the managed index region.
    pub index_entries: usize,
    /// Memories reviewed by the LLM pass this run.
    pub llm_reviewed: usize,
}

/// Tuning knobs, resolved from `KYMA_CC_*` env by callers that want env
/// configuration.
#[derive(Debug, Clone)]
pub struct CurationConfig {
    /// Master switch for promotion (curation of existing files always runs).
    pub promote: bool,
    /// Hard cap on managed `MEMORY.md` entries — the anti-flood invariant.
    pub promote_max: usize,
    /// Importance floor for promotion.
    pub promote_min_importance: f32,
    /// Plan only: emit actions but skip every DB mutation (stamps, merges),
    /// so a later real pass replans identically.
    pub dry_run: bool,
}

impl Default for CurationConfig {
    fn default() -> Self {
        CurationConfig {
            promote: true,
            promote_max: 15,
            promote_min_importance: 0.6,
            dry_run: false,
        }
    }
}

impl CurationConfig {
    /// Resolve from `KYMA_CC_PROMOTE`, `KYMA_CC_PROMOTE_MAX`,
    /// `KYMA_CC_PROMOTE_MIN_IMPORTANCE`.
    pub fn from_env() -> Self {
        let d = CurationConfig::default();
        let get = |k: &str| std::env::var(k).ok();
        CurationConfig {
            promote: get("KYMA_CC_PROMOTE").map_or(d.promote, |v| v != "0"),
            promote_max: get("KYMA_CC_PROMOTE_MAX")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.promote_max),
            promote_min_importance: get("KYMA_CC_PROMOTE_MIN_IMPORTANCE")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.promote_min_importance),
            dry_run: false,
        }
    }
}

/// A deferred "this file action has been handled" stamp.
///
/// Plans must not record apply-state at plan time: the applier can defer
/// (quiet window, lock) or fail, and a pre-committed stamp would make every
/// later plan skip the action forever — a silently lost promotion/archive.
/// The pipeline commits these only for actions the applier confirms applied;
/// until then each pass re-plans the action (superseded-propagation and
/// demotion are the natural retry loops).
#[derive(Debug, Clone)]
pub struct GuardStamp {
    /// Index into the plan's actions this stamp is contingent on.
    pub action: usize,
    pub node_id: String,
    /// Provenance keys to set.
    pub set: Vec<(String, Value)>,
    /// Provenance keys to remove.
    pub remove: Vec<String>,
}

/// Commit the guard stamps whose actions actually applied. Returns how many
/// were committed.
pub async fn commit_guard_stamps(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    stamps: &[GuardStamp],
    applied: &[bool],
    now: &str,
) -> anyhow::Result<usize> {
    let mut committed = 0;
    for s in stamps {
        if !applied.get(s.action).copied().unwrap_or(false) {
            continue;
        }
        stamp(writer, shared, &s.node_id, now, |p| {
            for (k, v) in &s.set {
                p.insert(k.clone(), v.clone());
            }
            for k in &s.remove {
                p.remove(k.as_str());
            }
        })
        .await?;
        committed += 1;
    }
    Ok(committed)
}

/// What to curate: one project realm.
#[derive(Debug, Clone)]
pub struct CurationInput<'a> {
    /// Memory realm (project basename).
    pub realm: &'a str,
    /// Claude Code path slug of the project dir.
    pub path_slug: &'a str,
    /// Pass timestamp (RFC3339) — injected for determinism.
    pub now: &'a str,
}

// ── LLM curation pass (gated on a usable engine) ────────────────────────────

/// What the model decided for one reviewed memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurationOp {
    Keep,
    Archive,
    Refresh,
}

/// Parsed LLM curation verdict for one stale-candidate memory.
#[derive(Debug, Clone)]
pub struct CurationDecision {
    pub op: CurationOp,
    /// Rewritten one-line description (for `Refresh`).
    pub refreshed_description: Option<String>,
    pub reason: Option<String>,
}

/// Knobs for the LLM pass.
#[derive(Debug, Clone)]
pub struct LlmCurationConfig {
    /// A memory must be at least this old (and unreviewed for as long)
    /// before it is questioned.
    pub stale_days: i64,
    /// Cosine-similarity band where duplication is plausible but not
    /// certain — the model decides. At/above the top of the band the
    /// deterministic pass would have merged already (exact dups).
    pub dup_band: (f64, f64),
    /// Plan only: skip every DB mutation.
    pub dry_run: bool,
}

impl Default for LlmCurationConfig {
    fn default() -> Self {
        LlmCurationConfig {
            stale_days: 90,
            dup_band: (0.90, 0.97),
            dry_run: false,
        }
    }
}

impl LlmCurationConfig {
    /// Resolve from `KYMA_CC_STALE_DAYS` / `KYMA_CC_DUP_COSINE`.
    pub fn from_env() -> Self {
        let d = LlmCurationConfig::default();
        LlmCurationConfig {
            stale_days: std::env::var("KYMA_CC_STALE_DAYS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.stale_days),
            dup_band: (
                d.dup_band.0,
                std::env::var("KYMA_CC_DUP_COSINE")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(d.dup_band.1),
            ),
            dry_run: false,
        }
    }
}

const CURATION_SYSTEM: &str = r#"You curate an AI agent's long-term memory for relevance. Given one MEMORY (with its age and type), decide whether it still earns a slot in the agent's always-loaded context. Return STRICT JSON:
{ "op": "KEEP | ARCHIVE | REFRESH", "refreshed_description": "rewritten one-line description (for REFRESH)", "reason": "one short sentence" }

Choose:
- KEEP: still accurate and useful as written.
- ARCHIVE: stale, no longer relevant, or clearly superseded. Archiving is reversible — but prefer KEEP when uncertain.
- REFRESH: still relevant but the description reads outdated — supply refreshed_description.

Output ONLY the JSON object."#;

#[derive(Debug, serde::Deserialize)]
struct RawCuration {
    #[serde(default)]
    op: String,
    #[serde(default)]
    refreshed_description: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// Parse a curation verdict, tolerantly. Anything unparseable degrades to
/// `Keep` — the safe default for an autonomous archiver.
pub fn parse_curation_decision(text: &str) -> CurationDecision {
    let keep = CurationDecision {
        op: CurationOp::Keep,
        refreshed_description: None,
        reason: None,
    };
    let Some(cleaned) = super::memory_extract::extract_json_object(text) else {
        return keep;
    };
    let Ok(raw) = serde_json::from_str::<RawCuration>(&cleaned) else {
        return keep;
    };
    let op = match raw.op.trim().to_ascii_uppercase().as_str() {
        "ARCHIVE" => CurationOp::Archive,
        "REFRESH" => CurationOp::Refresh,
        _ => CurationOp::Keep,
    };
    CurationDecision {
        op,
        refreshed_description: raw.refreshed_description.filter(|s| !s.trim().is_empty()),
        reason: raw.reason.filter(|s| !s.trim().is_empty()),
    }
}

/// Cosine similarity of two equal-length vectors (0.0 for zero vectors).
pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| f64::from(*x) * f64::from(*y)).sum();
    let norm = |v: &[f32]| v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let (na, nb) = (norm(a), norm(b));
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

/// Old enough to question, and not recently reviewed.
pub(crate) fn is_stale(
    updated_at: &str,
    reviewed_at: Option<&str>,
    now: &str,
    stale_days: i64,
) -> bool {
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
    let Some(now_t) = parse(now) else {
        return false;
    };
    let cutoff = now_t - chrono::Duration::days(stale_days);
    if parse(updated_at).is_none_or(|t| t >= cutoff) {
        return false;
    }
    match reviewed_at.and_then(parse) {
        Some(reviewed) => reviewed < cutoff,
        None => true,
    }
}

/// The LLM curation pass: stale review (KEEP/ARCHIVE/REFRESH) and
/// near-duplicate adjudication over the band the deterministic pass cannot
/// decide. Runs only when a usable engine is configured (local default:
/// none → clean no-op).
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn llm_curation_pass(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    engine: Option<&super::AgentState>,
    input: &CurationInput<'_>,
    cfg: &LlmCurationConfig,
    actions: &mut Vec<FileAction>,
    stamps: &mut Vec<GuardStamp>,
    outcome: &mut CurationOutcome,
) -> anyhow::Result<()> {
    let Some(state) = engine else {
        return Ok(());
    };
    // Mirror the consolidator's gate: a configured engine that adk-rust can
    // drive (ClaudeCli is CLI-spawned, not usable for these turns).
    let usable = matches!(
        state.engines.get().await,
        Ok(cfg) if cfg.kind != super::engine::EngineKind::ClaudeCli
    );
    if !usable {
        return Ok(());
    }

    let nodes = realm_nodes(shared, input.realm).await;
    let file_prefix = format!("{}{}/", kyma_ccmem::TOPIC_KEY_PREFIX, input.path_slug);
    let managed_file = |n: &Node| -> Option<String> {
        n.prov_str("cc_file")
            .or_else(|| n.prov_str("cc_promoted_file"))
            .map(str::to_string)
    };

    // ── Stale review: one verdict per long-unreviewed managed memory. ──
    let candidates: Vec<&Node> = nodes
        .iter()
        .filter(|n| {
            !n.dead()
                && !n.prov_flag("cc_user_owned")
                && (n.topic_key.starts_with(&file_prefix)
                    || n.prov_str("cc_promoted_file").is_some())
                && is_stale(
                    &n.updated_at,
                    n.prov_str("cc_reviewed_at"),
                    input.now,
                    cfg.stale_days,
                )
        })
        .collect();
    for n in candidates {
        let item = format!(
            "MEMORY (type={}, title={:?}, last_updated={}):\n{}",
            n.memory_type, n.title, n.updated_at, n.content
        );
        let Ok(text) = super::runner::run_oneshot(
            state,
            "kyma-memory-curator",
            "Reviews stale memories: KEEP / ARCHIVE / REFRESH.",
            CURATION_SYSTEM,
            &item,
        )
        .await
        else {
            continue; // engine hiccup: skip, never block the pass
        };
        let d = parse_curation_decision(&text);
        outcome.llm_reviewed += 1;
        match d.op {
            CurationOp::Keep => {
                if !cfg.dry_run {
                    stamp(writer, shared, &n.id, input.now, |p| {
                        p.insert("cc_reviewed_at".into(), serde_json::json!(input.now));
                    })
                    .await?;
                }
            }
            CurationOp::Archive => {
                if let Some(file) = managed_file(n) {
                    actions.push(FileAction::ArchiveFile {
                        file,
                        reason: d
                            .reason
                            .clone()
                            .unwrap_or_else(|| "stale (llm review)".to_string()),
                        node_id: Some(n.id.clone()),
                    });
                    stamps.push(GuardStamp {
                        action: actions.len() - 1,
                        node_id: n.id.clone(),
                        set: vec![("cc_file_archived".to_string(), serde_json::json!(true))],
                        remove: vec![
                            "cc_promoted_file".to_string(),
                            "cc_content_hash".to_string(),
                        ],
                    });
                    outcome.archived_files += 1;
                }
                // The node archive itself is knowledge — commits now; the
                // file-handled guard above commits only once the file lands
                // (superseded-propagation / demotion re-emit until then).
                if !cfg.dry_run {
                    archive_stale(writer, shared, n, d.reason.as_deref(), input.now).await?;
                }
            }
            CurationOp::Refresh => {
                if cfg.dry_run {
                } else if let Some(desc) = d.refreshed_description {
                    refresh_title(writer, shared, &n.id, &desc, input.now).await?;
                } else {
                    stamp(writer, shared, &n.id, input.now, |p| {
                        p.insert("cc_reviewed_at".into(), serde_json::json!(input.now));
                    })
                    .await?;
                }
            }
        }
    }

    let live: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.topic_key.starts_with(&file_prefix) && !n.dead())
        .collect();
    adjudicate_near_dups(shared, writer, state, input, cfg, &live, actions, stamps, outcome).await
}

/// Near-duplicate adjudication over the uncertainty band: the model decides
/// merge vs distinct for cosine-similar file-born pairs.
#[allow(clippy::too_many_arguments)]
async fn adjudicate_near_dups(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    state: &super::AgentState,
    input: &CurationInput<'_>,
    cfg: &LlmCurationConfig,
    live: &[&Node],
    actions: &mut Vec<FileAction>,
    stamps: &mut Vec<GuardStamp>,
    outcome: &mut CurationOutcome,
) -> anyhow::Result<()> {
    use super::memory_extract::ConflictOp;
    if live.len() < 2 {
        return Ok(());
    }
    let mut vecs: Vec<Vec<f32>> = Vec::with_capacity(live.len());
    for n in live {
        match writer.embed_one(&n.content).await {
            Ok(v) => vecs.push(v),
            Err(_) => return Ok(()), // no embedding backend → skip the band
        }
    }
    for i in 0..live.len() {
        for j in (i + 1)..live.len() {
            let sim = cosine(&vecs[i], &vecs[j]);
            if sim < cfg.dup_band.0 || sim >= cfg.dup_band.1 {
                continue;
            }
            // Pair already adjudicated as distinct?
            let (a, b) = if live[i].id <= live[j].id {
                (live[i], live[j])
            } else {
                (live[j], live[i])
            };
            let seen = a
                .prov
                .get("cc_dup_distinct")
                .and_then(Value::as_array)
                .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(b.id.as_str())));
            if seen {
                continue;
            }
            let Ok(d) = super::memory_extract::decide_conflict(
                state,
                &a.content,
                &[(b.id.clone(), b.content.clone())],
            )
            .await
            else {
                continue;
            };
            outcome.llm_reviewed += 1;
            if matches!(d.op, ConflictOp::Noop | ConflictOp::Update) {
                // Same knowledge: lower importance loses, exactly like the
                // deterministic exact-dup merge.
                let (winner, loser) = if a.importance >= b.importance { (a, b) } else { (b, a) };
                if !cfg.dry_run {
                    archive_as_duplicate(writer, shared, loser, winner, input.now).await?;
                    writer
                        .link(
                            &loser.id,
                            &winner.id,
                            kyma_memory::EDGE_MERGED_INTO,
                            input.realm,
                            None,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("merge edge: {e}"))?;
                }
                if let Some(file) = loser.prov_str("cc_file") {
                    actions.push(FileAction::ArchiveFile {
                        file: file.to_string(),
                        reason: format!("duplicate of {}", winner.display_title()),
                        node_id: Some(loser.id.clone()),
                    });
                    stamps.push(GuardStamp {
                        action: actions.len() - 1,
                        node_id: loser.id.clone(),
                        set: vec![("cc_file_archived".to_string(), serde_json::json!(true))],
                        remove: vec![],
                    });
                    outcome.archived_files += 1;
                }
                outcome.merged += 1;
            } else if !cfg.dry_run {
                // Distinct: remember the verdict so the pair is asked once.
                let b_id = b.id.clone();
                stamp(writer, shared, &a.id, input.now, move |p| {
                    let list = p
                        .entry("cc_dup_distinct")
                        .or_insert_with(|| serde_json::json!([]));
                    if let Some(arr) = list.as_array_mut() {
                        arr.push(serde_json::json!(b_id));
                    }
                })
                .await?;
            }
        }
    }
    Ok(())
}

/// Archive a managed memory the LLM judged stale (same-pass DB mirror).
async fn archive_stale(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    node: &Node,
    reason: Option<&str>,
    now: &str,
) -> anyhow::Result<()> {
    let Some(mut row) = fetch_full_row(shared, &node.id).await else {
        return Ok(());
    };
    // NOTE: the file-handled guard (`cc_file_archived`, promotion-stamp
    // clears) is committed post-apply via GuardStamp — a deferred archive
    // re-emits through the superseded/demotion retry loops.
    let mut prov = node.prov.clone();
    if let Some(obj) = prov.as_object_mut() {
        obj.insert(
            "cc_archived_reason".into(),
            serde_json::json!(reason.unwrap_or("stale (llm review)")),
        );
        obj.insert("cc_archived_at".into(), serde_json::json!(now));
    }
    if let Some(obj) = row.as_object_mut() {
        obj.insert("status".into(), serde_json::json!("archived"));
        obj.insert("invalid_at".into(), serde_json::json!(now));
        obj.insert("updated_at".into(), serde_json::json!(now));
        obj.insert("provenance".into(), serde_json::json!(prov.to_string()));
    }
    writer
        .append_node_rows(vec![row])
        .await
        .map_err(|e| anyhow::anyhow!("archiving stale {}: {e}", node.id))
}

/// Apply a refreshed description: new title + cleared content-hash stamp so
/// the promoted file re-renders on the next deterministic pass.
async fn refresh_title(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    node_id: &str,
    new_title: &str,
    now: &str,
) -> anyhow::Result<()> {
    let Some(mut row) = fetch_full_row(shared, node_id).await else {
        return Ok(());
    };
    let mut prov = row
        .get("provenance")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = prov.as_object_mut() {
        obj.insert("cc_reviewed_at".into(), serde_json::json!(now));
        obj.remove("cc_content_hash"); // force a file re-render next pass
    }
    if let Some(obj) = row.as_object_mut() {
        obj.insert("title".into(), serde_json::json!(new_title));
        obj.insert("updated_at".into(), serde_json::json!(now));
        obj.insert("provenance".into(), serde_json::json!(prov.to_string()));
    }
    writer
        .append_node_rows(vec![row])
        .await
        .map_err(|e| anyhow::anyhow!("refreshing {node_id}: {e}"))
}

/// A node's latest version, parsed out of the row JSON.
#[derive(Debug, Clone)]
struct Node {
    id: String,
    memory_type: String,
    title: Option<String>,
    content: String,
    importance: f64,
    status: String,
    invalid_at: String,
    topic_key: String,
    updated_at: String,
    prov: Value,
}

impl Node {
    fn from_row(row: &Value) -> Option<Node> {
        let get = |k: &str| row.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Node {
            id: get("id")?,
            memory_type: get("memory_type").unwrap_or_default(),
            title: get("title").filter(|t| !t.is_empty()),
            content: get("content").unwrap_or_default(),
            importance: row.get("importance").and_then(Value::as_f64).unwrap_or(0.5),
            status: get("status").unwrap_or_else(|| "active".to_string()),
            invalid_at: get("invalid_at").unwrap_or_default(),
            topic_key: get("topic_key").unwrap_or_default(),
            updated_at: get("updated_at").unwrap_or_default(),
            prov: get("provenance")
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_else(|| serde_json::json!({})),
        })
    }

    fn prov_str(&self, key: &str) -> Option<&str> {
        self.prov.get(key).and_then(Value::as_str)
    }

    fn prov_flag(&self, key: &str) -> bool {
        self.prov.get(key).and_then(Value::as_bool) == Some(true)
    }

    fn dead(&self) -> bool {
        self.status == "archived" || !self.invalid_at.is_empty()
    }

    fn display_title(&self) -> String {
        self.title.clone().unwrap_or_else(|| {
            self.content
                .split_whitespace()
                .take(6)
                .collect::<Vec<_>>()
                .join(" ")
        })
    }
}

const ALL_COLS: &str = "id, labels, realm, memory_type, title, content, content_preview, tags, \
    importance, status, source_session_id, source_run_id, embedding, created_at, updated_at, \
    valid_at, invalid_at, superseded_by, provenance, topic_key";

/// Plan one curation pass for a realm: file actions for the applier,
/// guard stamps for the pipeline to commit post-apply, and outcome
/// counters. Knowledge-level DB effects (duplicate merges, archive rows)
/// commit here; file-handled guards commit only after their action lands.
#[allow(clippy::too_many_lines)]
pub async fn plan_curation(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    input: &CurationInput<'_>,
    cfg: &CurationConfig,
) -> anyhow::Result<(Vec<FileAction>, Vec<GuardStamp>, CurationOutcome)> {
    let mut actions: Vec<FileAction> = Vec::new();
    let mut stamps: Vec<GuardStamp> = Vec::new();
    let mut outcome = CurationOutcome::default();
    let nodes = realm_nodes(shared, input.realm).await;
    let file_prefix = format!("{}{}/", kyma_ccmem::TOPIC_KEY_PREFIX, input.path_slug);

    // ── Superseded/archived file-born memories → archive their files (once).
    for n in nodes.iter().filter(|n| n.topic_key.starts_with(&file_prefix)) {
        if !n.dead()
            || n.prov_str("cc_archived_reason") == Some("file_deleted")
            || n.prov_flag("cc_file_archived")
        {
            continue;
        }
        let Some(file) = n.prov_str("cc_file").map(str::to_string) else {
            continue;
        };
        actions.push(FileAction::ArchiveFile {
            file,
            reason: "superseded in kyma".to_string(),
            node_id: Some(n.id.clone()),
        });
        stamps.push(GuardStamp {
            action: actions.len() - 1,
            node_id: n.id.clone(),
            set: vec![("cc_file_archived".to_string(), serde_json::json!(true))],
            remove: vec![],
        });
        outcome.archived_files += 1;
    }

    // ── Exact-duplicate merge among live file-born memories.
    {
        use std::collections::HashMap;
        let mut by_content: HashMap<String, Vec<&Node>> = HashMap::new();
        for n in nodes
            .iter()
            .filter(|n| n.topic_key.starts_with(&file_prefix) && !n.dead())
        {
            by_content
                .entry(n.content.trim().to_string())
                .or_default()
                .push(n);
        }
        let mut groups: Vec<Vec<&Node>> =
            by_content.into_values().filter(|g| g.len() > 1).collect();
        groups.sort_by(|a, b| a[0].topic_key.cmp(&b[0].topic_key));
        for mut group in groups {
            group.sort_by(|a, b| {
                b.importance
                    .total_cmp(&a.importance)
                    .then_with(|| a.topic_key.cmp(&b.topic_key))
            });
            let winner = group[0];
            for loser in &group[1..] {
                if !cfg.dry_run {
                    archive_as_duplicate(writer, shared, loser, winner, input.now).await?;
                    writer
                        .link(
                            &loser.id,
                            &winner.id,
                            kyma_memory::EDGE_MERGED_INTO,
                            input.realm,
                            None,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("merge edge: {e}"))?;
                }
                if let Some(file) = loser.prov_str("cc_file") {
                    actions.push(FileAction::ArchiveFile {
                        file: file.to_string(),
                        reason: format!("duplicate of {}", winner.display_title()),
                        node_id: Some(loser.id.clone()),
                    });
                    stamps.push(GuardStamp {
                        action: actions.len() - 1,
                        node_id: loser.id.clone(),
                        set: vec![("cc_file_archived".to_string(), serde_json::json!(true))],
                        remove: vec![],
                    });
                    outcome.archived_files += 1;
                }
                outcome.merged += 1;
            }
        }
    }

    // ── Promotion: high-value kyma memories become native files + the
    //    managed MEMORY.md region.
    if cfg.promote {
        promote(shared, input, cfg, &nodes, &mut actions, &mut stamps, &mut outcome).await;
    }

    Ok((actions, stamps, outcome))
}

#[allow(clippy::too_many_lines)]
async fn promote(
    shared: &SharedToolCtx,
    input: &CurationInput<'_>,
    cfg: &CurationConfig,
    nodes: &[Node],
    actions: &mut Vec<FileAction>,
    stamps: &mut Vec<GuardStamp>,
    outcome: &mut CurationOutcome,
) {
    let floor = f64::from(cfg.promote_min_importance);
    let is_file_born = |n: &Node| n.topic_key.starts_with(kyma_ccmem::TOPIC_KEY_PREFIX);
    let never_promotes =
        |n: &Node| matches!(n.memory_type.as_str(), "summary" | "entity") || is_file_born(n);

    // User-owned promoted files: the user edited them, kyma never rewrites —
    // but they stay indexed (they exist on disk and matter to the user).
    let user_owned: Vec<&Node> = nodes
        .iter()
        .filter(|n| n.prov_flag("cc_user_owned") && n.prov_str("cc_promoted_file").is_some())
        .collect();

    let prev_promoted: Vec<&Node> = nodes
        .iter()
        .filter(|n| {
            n.prov_str("cc_promoted_file").is_some()
                && !n.prov_flag("cc_user_owned")
                && !is_file_born(n)
        })
        .collect();

    // Demote what no longer belongs: invalidated/archived, or importance far
    // below the floor (hysteresis: 0.8 × floor).
    let mut demoted: Vec<(&Node, &str)> = Vec::new();
    let mut kept_prev: Vec<&Node> = Vec::new();
    for n in &prev_promoted {
        if n.dead() {
            demoted.push((n, "invalidated in kyma"));
        } else if n.importance < floor * 0.8 || never_promotes(n) {
            demoted.push((n, "demoted (importance fell)"));
        } else {
            kept_prev.push(n);
        }
    }

    let refs = reference_counts(shared, input.realm).await;
    let score = |n: &Node| -> f64 {
        #[allow(clippy::cast_precision_loss)] // tiny counts, capped at 3 anyway
        let refs_n = refs.get(&n.id).copied().unwrap_or(0) as f64;
        0.45 * n.importance
            + 0.20 * recency(&n.updated_at, input.now)
            + 0.25 * type_weight(&n.memory_type)
            + 0.10 * (refs_n / 3.0).min(1.0)
    };

    // Selection: user-owned hold their slots, then surviving promotions
    // (hysteresis), then new candidates by score, hard-capped.
    let slots = cfg.promote_max.saturating_sub(user_owned.len());
    kept_prev.sort_by(|a, b| score(b).total_cmp(&score(a)));
    while kept_prev.len() > slots {
        let n = kept_prev.pop().expect("non-empty");
        demoted.push((n, "demoted (over index cap)"));
    }
    let mut candidates: Vec<&Node> = nodes
        .iter()
        .filter(|n| {
            !n.dead()
                && !never_promotes(n)
                && n.importance >= floor
                && n.prov_str("cc_promoted_file").is_none()
                && !n.prov_flag("cc_user_owned")
        })
        .collect();
    candidates.sort_by(|a, b| score(b).total_cmp(&score(a)).then_with(|| a.id.cmp(&b.id)));
    let mut selected: Vec<&Node> = kept_prev;
    for c in candidates {
        if selected.len() >= slots {
            break;
        }
        selected.push(c);
    }
    selected.sort_by(|a, b| score(b).total_cmp(&score(a)).then_with(|| a.id.cmp(&b.id)));

    // Apply demotions: archive file + clear the stamp once that lands.
    for (n, reason) in demoted {
        if let Some(file) = n.prov_str("cc_promoted_file").map(str::to_string) {
            actions.push(FileAction::ArchiveFile {
                file,
                reason: reason.to_string(),
                node_id: Some(n.id.clone()),
            });
            stamps.push(GuardStamp {
                action: actions.len() - 1,
                node_id: n.id.clone(),
                set: vec![],
                remove: vec!["cc_promoted_file".to_string(), "cc_content_hash".to_string()],
            });
            outcome.archived_files += 1;
        }
    }

    // Related wikilinks: direct edges between selected memories.
    let related = related_names(shared, input.realm, &selected).await;

    let mut used_files: std::collections::HashSet<String> = user_owned
        .iter()
        .filter_map(|n| n.prov_str("cc_promoted_file").map(str::to_string))
        .collect();
    let mut entries: Vec<IndexEntry> = Vec::new();
    for n in &selected {
        let title = n.display_title();
        let file = match n.prov_str("cc_promoted_file") {
            Some(f) => f.to_string(),
            None => unique_file(&kyma_ccmem::slug::memory_filename(&title), &used_files),
        };
        used_files.insert(file.clone());
        let name = file.trim_end_matches(".md").to_string();
        let cc_type = cc_type_for(&n.memory_type);
        let body = build_body(&n.content, related.get(&n.id).map_or(&[][..], |v| v));
        let hash = kyma_ccmem::hash::content_hash(&name, Some(cc_type), &body);

        let unchanged = n.prov_str("cc_promoted_file") == Some(file.as_str())
            && n.prov_str("cc_content_hash") == Some(hash.as_str());
        if !unchanged {
            let rendered = kyma_ccmem::frontmatter::render(&kyma_ccmem::frontmatter::MemoryFile {
                front: kyma_ccmem::frontmatter::Frontmatter {
                    name: Some(name.clone()),
                    description: Some(clip(&title, 140)),
                    cc_type: Some(cc_type.to_string()),
                    source: Some(kyma_ccmem::KYMA_SOURCE_MARKER.to_string()),
                    kyma_memory_id: Some(n.id.clone()),
                    content_hash: Some(hash.clone()),
                    ..kyma_ccmem::frontmatter::Frontmatter::default()
                },
                body: body.clone(),
            });
            if n.prov_str("cc_promoted_file").is_none() {
                outcome.promoted += 1;
            } else {
                outcome.refreshed += 1;
            }
            actions.push(FileAction::WriteMemoryFile {
                file: file.clone(),
                content: rendered,
                node_id: n.id.clone(),
                content_hash: hash.clone(),
            });
            stamps.push(GuardStamp {
                action: actions.len() - 1,
                node_id: n.id.clone(),
                set: vec![
                    ("cc_promoted_file".to_string(), serde_json::json!(file)),
                    ("cc_content_hash".to_string(), serde_json::json!(hash)),
                    ("cc_promoted_at".to_string(), serde_json::json!(input.now)),
                ],
                remove: vec![],
            });
        }
        entries.push(IndexEntry {
            title: title.clone(),
            file,
            hook: clip(&title, 80),
        });
    }
    for n in &user_owned {
        let Some(file) = n.prov_str("cc_promoted_file") else {
            continue;
        };
        let title = n.display_title();
        entries.push(IndexEntry {
            title: title.clone(),
            file: file.to_string(),
            hook: clip(&title, 80),
        });
    }

    outcome.index_entries = entries.len();
    actions.push(FileAction::SetIndex { entries });
}

/// Reverse of the ingest mapping: kyma memory type → Claude Code
/// `metadata.type`.
pub fn cc_type_for(memory_type: &str) -> &'static str {
    match memory_type {
        "preference" => "user",
        "learning" => "feedback",
        "decision" => "project",
        _ => "reference",
    }
}

fn type_weight(t: &str) -> f64 {
    match t {
        "decision" | "preference" | "procedure" => 1.0,
        "learning" => 0.8,
        "fact" => 0.6,
        _ => 0.0,
    }
}

/// Exponential decay with kyma's standard 30-day half-life.
fn recency(updated_at: &str, now: &str) -> f64 {
    let parse = |s: &str| chrono::DateTime::parse_from_rfc3339(s).ok();
    let (Some(u), Some(n)) = (parse(updated_at), parse(now)) else {
        return 0.5;
    };
    #[allow(clippy::cast_precision_loss)] // second counts fit f64's mantissa
    let age_days = (n - u).num_seconds().max(0) as f64 / 86_400.0;
    (-age_days / kyma_memory::HALF_LIFE_DAYS).exp2()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn unique_file(base: &str, used: &std::collections::HashSet<String>) -> String {
    if !used.contains(base) {
        return base.to_string();
    }
    let stem = base.trim_end_matches(".md");
    for i in 2.. {
        let candidate = format!("{stem}-{i}.md");
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn build_body(content: &str, related: &[String]) -> String {
    let mut body = content.trim_end().to_string();
    if !related.is_empty() {
        body.push_str("\n\nRelated: ");
        let links: Vec<String> = related
            .iter()
            .map(|n| kyma_ccmem::wikilink::to_wikilink(n))
            .collect();
        body.push_str(&links.join(", "));
    }
    body.push_str("\n\n<!-- managed by kyma — edit freely; kyma pulls your edits back -->\n");
    body
}

/// Latest version of every node in the realm.
async fn realm_nodes(shared: &SharedToolCtx, realm: &str) -> Vec<Node> {
    let q = format!(
        "WITH latest AS (SELECT *, \
           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM {nt}) \
         SELECT id, memory_type, title, content, importance, status, updated_at, \
                invalid_at, topic_key, provenance \
         FROM latest WHERE rn = 1 AND realm = {r}",
        nt = kyma_memory::NODE_TABLE,
        r = kyma_memory::sql::sql_str(realm),
    );
    let res = super::execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 100_000).await;
    res.get("rows")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().filter_map(Node::from_row).collect())
        .unwrap_or_default()
}

/// `REFERENCES` out-degree per node — the graph-corroboration signal.
async fn reference_counts(
    shared: &SharedToolCtx,
    realm: &str,
) -> std::collections::HashMap<String, i64> {
    let q = format!(
        "SELECT src, COUNT(DISTINCT id) AS c FROM {et} \
         WHERE type = 'REFERENCES' AND realm = {r} GROUP BY src",
        et = kyma_memory::EDGE_TABLE,
        r = kyma_memory::sql::sql_str(realm),
    );
    let res = super::execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 100_000).await;
    res.get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| {
                    Some((
                        r.get("src")?.as_str()?.to_string(),
                        r.get("c").and_then(Value::as_i64).unwrap_or(0),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// For each selected node: stems of other selected files it is directly
/// edge-linked to (`RELATES_TO` / `REFERENCES`, either direction), capped at 3.
async fn related_names(
    shared: &SharedToolCtx,
    realm: &str,
    selected: &[&Node],
) -> std::collections::HashMap<String, Vec<String>> {
    use std::collections::HashMap;
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    if selected.len() < 2 {
        return out;
    }
    let stems: HashMap<&str, String> = selected
        .iter()
        .filter_map(|n| {
            n.prov_str("cc_promoted_file")
                .map(|f| (n.id.as_str(), f.trim_end_matches(".md").to_string()))
        })
        .collect();
    let q = format!(
        "SELECT DISTINCT src, dst FROM {et} \
         WHERE type IN ('RELATES_TO', 'REFERENCES') AND realm = {r}",
        et = kyma_memory::EDGE_TABLE,
        r = kyma_memory::sql::sql_str(realm),
    );
    let res = super::execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 100_000).await;
    let Some(rows) = res.get("rows").and_then(Value::as_array) else {
        return out;
    };
    for row in rows {
        let (Some(src), Some(dst)) = (
            row.get("src").and_then(Value::as_str),
            row.get("dst").and_then(Value::as_str),
        ) else {
            continue;
        };
        for (a, b) in [(src, dst), (dst, src)] {
            if let Some(stem) = stems.get(b) {
                let v = out.entry(a.to_string()).or_default();
                if v.len() < 3 && !v.contains(stem) {
                    v.push(stem.clone());
                }
            }
        }
    }
    for v in out.values_mut() {
        v.sort();
    }
    out
}

/// Read-latest → mutate provenance → append a new version (the repo's
/// standard append-only mutation idiom).
async fn stamp(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    node_id: &str,
    now: &str,
    f: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> anyhow::Result<()> {
    let Some(mut row) = fetch_full_row(shared, node_id).await else {
        return Ok(());
    };
    let mut prov = row
        .get("provenance")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = prov.as_object_mut() {
        f(obj);
    }
    if let Some(obj) = row.as_object_mut() {
        obj.insert("provenance".into(), serde_json::json!(prov.to_string()));
        obj.insert("updated_at".into(), serde_json::json!(now));
    }
    writer
        .append_node_rows(vec![row])
        .await
        .map_err(|e| anyhow::anyhow!("stamping {node_id}: {e}"))
}

/// Archive `loser` as a duplicate of `winner` (same-pass DB mirror of the
/// file archive action).
async fn archive_as_duplicate(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    loser: &Node,
    winner: &Node,
    now: &str,
) -> anyhow::Result<()> {
    let Some(mut row) = fetch_full_row(shared, &loser.id).await else {
        return Ok(());
    };
    // NOTE: `cc_file_archived` is deliberately NOT set here — that guard is
    // committed post-apply (GuardStamp), so a deferred file archive re-emits
    // via superseded-propagation instead of orphaning the file.
    let mut prov = loser.prov.clone();
    if let Some(obj) = prov.as_object_mut() {
        obj.insert("cc_archived_reason".into(), serde_json::json!("duplicate"));
        obj.insert("cc_archived_at".into(), serde_json::json!(now));
    }
    if let Some(obj) = row.as_object_mut() {
        obj.insert("status".into(), serde_json::json!("archived"));
        obj.insert("invalid_at".into(), serde_json::json!(now));
        obj.insert("superseded_by".into(), serde_json::json!(winner.id));
        obj.insert("updated_at".into(), serde_json::json!(now));
        obj.insert("provenance".into(), serde_json::json!(prov.to_string()));
    }
    writer
        .append_node_rows(vec![row])
        .await
        .map_err(|e| anyhow::anyhow!("archiving duplicate {}: {e}", loser.id))
}

async fn fetch_full_row(shared: &SharedToolCtx, node_id: &str) -> Option<Value> {
    let q = format!(
        "WITH latest AS (SELECT *, \
           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM {nt}) \
         SELECT {ALL_COLS} FROM latest WHERE rn = 1 AND id = {id} LIMIT 1",
        nt = kyma_memory::NODE_TABLE,
        id = kyma_memory::sql::sql_str(node_id),
    );
    let res = super::execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 1).await;
    res.get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
}
