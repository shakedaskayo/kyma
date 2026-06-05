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
}

impl Default for CurationConfig {
    fn default() -> Self {
        CurationConfig {
            promote: true,
            promote_max: 15,
            promote_min_importance: 0.6,
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
        }
    }
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

/// Plan one curation pass for a realm. Returns the file actions for the
/// applier plus outcome counters. DB-side effects (promotion stamps,
/// duplicate merges, archive marks) are applied here, in the same pass.
pub async fn plan_curation(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    input: &CurationInput<'_>,
    cfg: &CurationConfig,
) -> anyhow::Result<(Vec<FileAction>, CurationOutcome)> {
    let mut actions: Vec<FileAction> = Vec::new();
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
        outcome.archived_files += 1;
        stamp(writer, shared, &n.id, input.now, |p| {
            p.insert("cc_file_archived".into(), serde_json::json!(true));
        })
        .await?;
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
                if let Some(file) = loser.prov_str("cc_file") {
                    actions.push(FileAction::ArchiveFile {
                        file: file.to_string(),
                        reason: format!("duplicate of {}", winner.display_title()),
                        node_id: Some(loser.id.clone()),
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
        promote(shared, writer, input, cfg, &nodes, &mut actions, &mut outcome).await?;
    }

    Ok((actions, outcome))
}

#[allow(clippy::too_many_lines)]
async fn promote(
    shared: &SharedToolCtx,
    writer: &MemoryWriter,
    input: &CurationInput<'_>,
    cfg: &CurationConfig,
    nodes: &[Node],
    actions: &mut Vec<FileAction>,
    outcome: &mut CurationOutcome,
) -> anyhow::Result<()> {
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

    // Apply demotions: archive file + clear the stamp so this happens once.
    for (n, reason) in demoted {
        if let Some(file) = n.prov_str("cc_promoted_file").map(str::to_string) {
            actions.push(FileAction::ArchiveFile {
                file,
                reason: reason.to_string(),
                node_id: Some(n.id.clone()),
            });
            outcome.archived_files += 1;
        }
        stamp(writer, shared, &n.id, input.now, |p| {
            p.remove("cc_promoted_file");
            p.remove("cc_content_hash");
        })
        .await?;
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
            stamp(writer, shared, &n.id, input.now, |p| {
                p.insert("cc_promoted_file".into(), serde_json::json!(file));
                p.insert("cc_content_hash".into(), serde_json::json!(hash));
                p.insert("cc_promoted_at".into(), serde_json::json!(input.now));
            })
            .await?;
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
    Ok(())
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
    let mut prov = loser.prov.clone();
    if let Some(obj) = prov.as_object_mut() {
        obj.insert("cc_archived_reason".into(), serde_json::json!("duplicate"));
        obj.insert("cc_file_archived".into(), serde_json::json!(true));
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
