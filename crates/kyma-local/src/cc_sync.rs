//! Ingest Claude Code's file-based memory into the local kyma engine.
//!
//! Claude Code persists per-project memories as markdown files under
//! `~/.claude/projects/<path-slug>/memory/` (see `kyma-ccmem` for the
//! format). This module scans those directories and upserts each memory
//! file into `memory_nodes` — embedded, graph-linked, recallable — with the
//! **files as source of truth**:
//!
//! - **Idempotent**: per-file normalized content hashes in `sync_state`
//!   (`ccsync:hash:<path>`) skip unchanged files without embedding; upserts
//!   key on `topic_key = claude-md:<slug>/<name>` so edits become new
//!   versions of the same node, never duplicates.
//! - **Loop-safe**: files kyma itself promoted (frontmatter
//!   `metadata.source: kyma`) are skipped while their on-disk hash matches
//!   the stamped `content_hash`; a mismatch means the *user* edited the file,
//!   which updates the original node and marks it user-owned so writeback
//!   stops overwriting it.
//! - **Realm**: the project's directory basename, matching the plugin hooks'
//!   `kyma_realm()` convention — file memories surface in normal recall.
//!   Slugs resolve to project paths via `~/.claude.json`'s `projects` keys,
//!   falling back to the `cwd` recorded in session transcripts.

use std::io::BufRead as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::Engine;
use kyma_ccmem::frontmatter::MemoryFile;
use kyma_ccmem::{frontmatter, hash, slug};
use kyma_memory::{CreateMemory, MemoryType, MemoryWriter};
use kyma_server::agent::{execute_sql, SharedToolCtx};

/// Importance assigned to file-born memories: Claude Code deliberately chose
/// to persist them, which puts them above the 0.5 default but below
/// explicitly high-stakes saves.
const FILE_IMPORTANCE: f32 = 0.7;

/// Where to scan and what to limit to. Resolved from env by the caller
/// (`~/.claude/projects`, `~/.claude.json`), injectable for tests.
pub(crate) struct CcSyncOptions {
    /// Root of Claude Code's per-project state (`~/.claude/projects`).
    pub projects_dir: PathBuf,
    /// `~/.claude.json` — its `projects` keys resolve slugs to real paths.
    pub claude_json: Option<PathBuf>,
    /// Only sync the project at this absolute path (all when `None`).
    pub project: Option<PathBuf>,
}

/// Per-project outcome, for logs/reports.
#[derive(Debug, Default)]
pub(crate) struct ProjectSyncReport {
    pub slug: String,
    pub realm: String,
    /// Files ingested or re-ingested (created or new version).
    pub upserted: usize,
    /// Files unchanged since the last scan (hash hit) or kyma-authored and
    /// untouched.
    pub skipped: usize,
    /// kyma-authored files the user edited — pulled back as node updates.
    pub user_edited: usize,
}

/// Outcome of one full scan.
#[derive(Debug, Default)]
pub(crate) struct CcSyncReport {
    pub projects: Vec<ProjectSyncReport>,
}

/// Scan every project memory dir under `opts.projects_dir` and sync each
/// memory file into the engine.
pub(crate) async fn run_once(
    engine: &Engine,
    writer: &MemoryWriter,
    opts: &CcSyncOptions,
) -> Result<CcSyncReport> {
    let known_paths = known_project_paths(opts.claude_json.as_deref());
    let only_slug = opts
        .project
        .as_deref()
        .map(|p| slug::path_slug(p));

    let mut slug_dirs: Vec<PathBuf> = Vec::new();
    let entries = match std::fs::read_dir(&opts.projects_dir) {
        Ok(e) => e,
        // No Claude Code state on this machine — a clean no-op, not an error.
        Err(_) => return Ok(CcSyncReport::default()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.join("memory").is_dir() {
            continue;
        }
        if let Some(only) = &only_slug {
            if path.file_name().and_then(|n| n.to_str()) != Some(only.as_str()) {
                continue;
            }
        }
        slug_dirs.push(path);
    }
    slug_dirs.sort();

    let mut report = CcSyncReport::default();
    for dir in slug_dirs {
        match sync_project(engine, writer, &dir, &known_paths).await {
            Ok(p) => report.projects.push(p),
            // One broken project must not block the rest.
            Err(e) => tracing::warn!(dir = %dir.display(), "cc-sync: project failed: {e}"),
        }
    }
    Ok(report)
}

/// Sync a single project's `memory/` directory.
async fn sync_project(
    engine: &Engine,
    writer: &MemoryWriter,
    slug_dir: &Path,
    known_paths: &[String],
) -> Result<ProjectSyncReport> {
    let slug = slug_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    let realm = resolve_realm(slug_dir, &slug, known_paths);
    let shared = SharedToolCtx {
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
    };
    let mut report = ProjectSyncReport {
        slug: slug.clone(),
        realm: realm.clone(),
        ..ProjectSyncReport::default()
    };

    let memory_dir = slug_dir.join("memory");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&memory_dir)
        .with_context(|| format!("reading {}", memory_dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().and_then(|e| e.to_str()) == Some("md")
                && p.file_name().and_then(|n| n.to_str()) != Some(kyma_ccmem::MEMORY_INDEX_FILE)
        })
        .collect();
    files.sort();

    let mut manifest: Vec<Value> = Vec::new();
    let now = chrono::Utc::now().to_rfc3339();

    for path in files {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(parsed) = frontmatter::parse(&raw) else {
            tracing::debug!(file = %path.display(), "cc-sync: no/invalid frontmatter, skipping");
            continue;
        };
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let name = parsed.front.name.clone().unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string()
        });
        let h = hash::content_hash(&name, parsed.front.cc_type.as_deref(), &parsed.body);
        let hash_key = format!("ccsync:hash:{}", path.display());

        if parsed.is_kyma_authored() {
            manifest.push(json!({
                "file": file_name,
                "name": name,
                "kyma": true,
                "node_id": parsed.front.kyma_memory_id,
            }));
            if parsed.front.content_hash.as_deref() == Some(h.as_str()) {
                // Our own promotion, untouched — never re-ingest (loop guard).
                report.skipped += 1;
                engine.sqlite.set_sync_state(&hash_key, &h).await?;
                continue;
            }
            // The user edited a kyma-promoted file: the file wins. Pull the
            // edit back into the original node and mark it user-owned so
            // writeback stops overwriting this file.
            if let Some(uuid) = parsed
                .front
                .kyma_memory_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id.strip_prefix("memory:").unwrap_or(id)).ok())
            {
                apply_user_edit(writer, &shared, uuid, &parsed, &realm, &h, &now).await?;
                report.user_edited += 1;
                engine.sqlite.set_sync_state(&hash_key, &h).await?;
                continue;
            }
            // No back-pointer — fall through and treat as a regular file.
        }

        if engine.sqlite.get_sync_state(&hash_key).await?.as_deref() == Some(h.as_str()) {
            report.skipped += 1;
            manifest.push(json!({
                "file": file_name,
                "name": name,
                "topic_key": kyma_ccmem::topic_key(&slug, &name),
            }));
            continue;
        }

        let tk = kyma_ccmem::topic_key(&slug, &name);
        let (memory_type, tags) = map_cc_type(parsed.front.cc_type.as_deref());
        let mut cm = CreateMemory::new(parsed.body.clone());
        cm.title = Some(
            parsed
                .front
                .description
                .clone()
                .unwrap_or_else(|| name.clone()),
        );
        cm.memory_type = memory_type;
        cm.tags = tags;
        cm.realm = realm.clone();
        cm.importance = FILE_IMPORTANCE;
        cm.source_session_id = parsed
            .front
            .origin_session_id
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok());
        cm.topic_key = Some(tk.clone());
        cm.provenance = Some(json!({
            "source": kyma_ccmem::CC_PROVENANCE_SOURCE,
            "cc_path_slug": slug,
            "cc_name": name,
            "cc_file": file_name,
            "cc_origin_session_id": parsed.front.origin_session_id,
            "content_hash": h,
            "ingested_at": now,
        }));

        if let Some(existing) = node_id_by_topic_key(&shared, &tk).await {
            let uuid_part = existing.strip_prefix("memory:").unwrap_or(&existing);
            if let Ok(u) = Uuid::parse_str(uuid_part) {
                writer
                    .save_as(u, &cm)
                    .await
                    .with_context(|| format!("upserting {}", path.display()))?;
            }
        } else {
            writer
                .save(&cm)
                .await
                .with_context(|| format!("saving {}", path.display()))?;
        }
        report.upserted += 1;
        engine.sqlite.set_sync_state(&hash_key, &h).await?;
        manifest.push(json!({
            "file": file_name,
            "name": name,
            "topic_key": tk,
        }));
    }

    engine
        .sqlite
        .set_sync_state(
            &format!("ccsync:manifest:{slug}"),
            &Value::Array(manifest).to_string(),
        )
        .await?;
    Ok(report)
}

/// Pull a user edit of a kyma-promoted file back into its node, preserving
/// the node's identity fields (title, type, tags, importance, topic_key).
async fn apply_user_edit(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    uuid: Uuid,
    parsed: &MemoryFile,
    realm: &str,
    new_hash: &str,
    now: &str,
) -> Result<()> {
    let prev = latest_node(shared, &format!("memory:{uuid}")).await;
    let mut cm = CreateMemory::new(parsed.body.clone());
    cm.realm = realm.to_string();
    cm.importance = FILE_IMPORTANCE;
    let mut prov = json!({});
    if let Some(prev) = &prev {
        if let Some(t) = prev.get("title").and_then(Value::as_str) {
            cm.title = Some(t.to_string());
        }
        if let Some(mt) = prev.get("memory_type").and_then(Value::as_str) {
            cm.memory_type = MemoryType::parse(mt);
        }
        if let Some(tags) = prev.get("tags").and_then(Value::as_str) {
            cm.tags = tags
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
        }
        if let Some(imp) = prev.get("importance").and_then(Value::as_f64) {
            #[allow(clippy::cast_possible_truncation)]
            {
                cm.importance = imp as f32;
            }
        }
        if let Some(tk) = prev.get("topic_key").and_then(Value::as_str) {
            cm.topic_key = Some(tk.to_string());
        }
        if let Some(r) = prev.get("realm").and_then(Value::as_str) {
            cm.realm = r.to_string();
        }
        if let Some(p) = prev
            .get("provenance")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
        {
            prov = p;
        }
    }
    cm.title = cm
        .title
        .or_else(|| parsed.front.description.clone())
        .or_else(|| parsed.front.name.clone());
    if let Some(obj) = prov.as_object_mut() {
        obj.insert("cc_user_owned".into(), json!(true));
        obj.insert("content_hash".into(), json!(new_hash));
        obj.insert("user_edited_at".into(), json!(now));
    }
    cm.provenance = Some(prov);
    writer
        .save_as(uuid, &cm)
        .await
        .with_context(|| format!("applying user edit to memory:{uuid}"))?;
    Ok(())
}

/// Map Claude Code's `metadata.type` onto kyma's memory types. `reference`
/// has no kyma variant, so it becomes a tagged fact.
fn map_cc_type(cc: Option<&str>) -> (MemoryType, Vec<String>) {
    match cc.map(str::trim) {
        Some("user") => (MemoryType::Preference, Vec::new()),
        Some("feedback") => (MemoryType::Learning, Vec::new()),
        Some("reference") => (MemoryType::Fact, vec!["reference".to_string()]),
        _ => (MemoryType::Fact, Vec::new()),
    }
}

/// The realm for a project dir: basename of the resolved project path
/// (matching the plugin hooks), falling back to the transcript `cwd`, then
/// to the slug itself.
fn resolve_realm(slug_dir: &Path, dir_slug: &str, known_paths: &[String]) -> String {
    if let Some(p) = slug::resolve_project_path(dir_slug, known_paths) {
        return slug::realm_for_path(&p);
    }
    if let Some(cwd) = transcript_cwd(slug_dir) {
        return slug::realm_for_path(&cwd);
    }
    dir_slug.to_string()
}

/// Read the `cwd` recorded in the first line of any session transcript in
/// the project dir — the fallback when `~/.claude.json` doesn't know the
/// project anymore.
fn transcript_cwd(slug_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(slug_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        let mut line = String::new();
        if std::io::BufReader::new(file).read_line(&mut line).is_err() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            if let Some(cwd) = v.get("cwd").and_then(Value::as_str) {
                return Some(PathBuf::from(cwd));
            }
        }
    }
    None
}

/// Project paths Claude Code knows about (`projects` keys of `~/.claude.json`).
fn known_project_paths(claude_json: Option<&Path>) -> Vec<String> {
    let Some(path) = claude_json else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| {
            v.get("projects")
                .and_then(Value::as_object)
                .map(|m| m.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// Find the live node carrying a topic key. Keys embed the project slug, so
/// they are globally unique — no realm filter (rename-safe).
async fn node_id_by_topic_key(shared: &SharedToolCtx, topic_key: &str) -> Option<String> {
    let q = format!(
        "WITH latest AS (SELECT id, topic_key, \
           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM {nt}) \
         SELECT id FROM latest WHERE rn = 1 AND topic_key = {tk} LIMIT 1",
        nt = kyma_memory::NODE_TABLE,
        tk = kyma_memory::sql::sql_str(topic_key),
    );
    let res = execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 1).await;
    res.get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Latest version of a node, full display row.
async fn latest_node(shared: &SharedToolCtx, node_id: &str) -> Option<Value> {
    let q = format!(
        "WITH latest AS (SELECT *, \
           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM {nt}) \
         SELECT id, realm, memory_type, title, tags, importance, topic_key, provenance \
         FROM latest WHERE rn = 1 AND id = {id} LIMIT 1",
        nt = kyma_memory::NODE_TABLE,
        id = kyma_memory::sql::sql_str(node_id),
    );
    let res = execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 1).await;
    res.get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
}
