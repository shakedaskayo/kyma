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

use crate::sync::NODE_COLS;
use crate::Engine;
use kyma_ccmem::frontmatter::MemoryFile;
use kyma_ccmem::{frontmatter, hash, slug, wikilink};
use kyma_memory::{rows::edge_row, CreateMemory, MemoryType, MemoryWriter};
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
    /// The Claude Code project dir (`~/.claude/projects/<slug>`).
    pub dir: PathBuf,
    /// Resolved absolute project path, when known.
    pub project_path: Option<PathBuf>,
    /// Files ingested or re-ingested (created or new version).
    pub upserted: usize,
    /// Files unchanged since the last scan (hash hit) or kyma-authored and
    /// untouched.
    pub skipped: usize,
    /// kyma-authored files the user edited — pulled back as node updates.
    pub user_edited: usize,
    /// `RELATES_TO` edges appended for resolved `[[wikilinks]]`.
    pub edges_added: usize,
    /// Nodes archived because their file disappeared from disk.
    pub archived: usize,
}

/// What one scanned (non-kyma) file contributed — input to the wikilink
/// edge pass and the deletion manifest.
struct ScanEntry {
    name: String,
    topic_key: String,
    /// Set when this run wrote the node (save/save_as); looked up lazily
    /// otherwise.
    node_id: Option<String>,
    wikilinks: Vec<String>,
    changed: bool,
}

/// Outcome of one full scan.
#[derive(Debug, Default)]
pub(crate) struct CcSyncReport {
    pub projects: Vec<ProjectSyncReport>,
}

impl CcSyncReport {
    /// Rollup as a `last_scan` JSON value in the control plane's `ScanStats`
    /// shape (`kyma_datasources::watchers`): `{seen, processed, errors,
    /// duration_ms, at, detail}` with `detail.realms` carrying per-project
    /// counts. Built manually rather than via Serialize: `ProjectSyncReport`
    /// embeds `PathBuf`s (`dir`, `project_path`) that don't belong on the
    /// wire, and the totals here aren't fields of the struct anyway.
    ///
    /// `errors` is always 0: per-project failures are logged and dropped
    /// inside `run_once` (one broken project must not block the rest), so
    /// nothing reaches this rollup; a whole-pass failure never produces a
    /// report at all.
    pub(crate) fn last_scan_value(
        &self,
        duration_ms: u64,
        at: chrono::DateTime<chrono::Utc>,
    ) -> Value {
        let (mut seen, mut processed) = (0usize, 0usize);
        let realms: Vec<Value> = self
            .projects
            .iter()
            .map(|p| {
                seen += p.upserted + p.skipped + p.user_edited;
                processed += p.upserted + p.user_edited;
                json!({
                    "realm": p.realm,
                    "upserted": p.upserted,
                    "skipped": p.skipped,
                    "user_edited": p.user_edited,
                    "edges_added": p.edges_added,
                    "archived": p.archived,
                })
            })
            .collect();
        json!({
            "seen": seen,
            "processed": processed,
            "errors": 0,
            "duration_ms": duration_ms,
            "at": at.to_rfc3339(),
            "detail": { "realms": realms },
        })
    }
}

/// Scan every project memory dir under `opts.projects_dir` and sync each
/// memory file into the engine.
pub(crate) async fn run_once(
    engine: &Engine,
    writer: &MemoryWriter,
    opts: &CcSyncOptions,
) -> Result<CcSyncReport> {
    let known_paths = known_project_paths(opts.claude_json.as_deref());
    let only_slug = opts.project.as_deref().map(|p| slug::path_slug(p));

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
    let (realm, project_path) = resolve_realm(slug_dir, &slug, known_paths);
    let shared = SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
        memory: None,
        hitl: None,
        memory_settings_path: None,
    };
    let mut report = ProjectSyncReport {
        slug: slug.clone(),
        realm: realm.clone(),
        dir: slug_dir.to_path_buf(),
        project_path,
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

    let manifest_key = format!("ccsync:manifest:{slug}");
    let old_manifest: Vec<Value> = engine
        .sqlite
        .get_sync_state(&manifest_key)
        .await?
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let mut manifest: Vec<Value> = Vec::new();
    let mut entries: Vec<ScanEntry> = Vec::new();
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
            // Scan state first: this exact content was already processed
            // (e.g. a user edit pulled back on a previous pass — the in-file
            // stamp never matches again by design, so it can't be the skip
            // signal here).
            if engine.sqlite.get_sync_state(&hash_key).await?.as_deref() == Some(h.as_str()) {
                report.skipped += 1;
                manifest.push(json!({
                    "file": file_name,
                    "name": name,
                    "kyma": true,
                    "node_id": parsed.front.kyma_memory_id,
                }));
                continue;
            }
            if parsed.front.content_hash.as_deref() == Some(h.as_str()) {
                // Our own promotion, untouched — never re-ingest (loop guard).
                report.skipped += 1;
                engine.sqlite.set_sync_state(&hash_key, &h).await?;
                manifest.push(json!({
                    "file": file_name,
                    "name": name,
                    "kyma": true,
                    "node_id": parsed.front.kyma_memory_id,
                }));
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
                manifest.push(json!({
                    "file": file_name,
                    "name": name,
                    "kyma": true,
                    "node_id": parsed.front.kyma_memory_id,
                }));
                continue;
            }
            // No back-pointer — fall through and treat as a regular file.
        }

        let tk = kyma_ccmem::topic_key(&slug, &name);
        if engine.sqlite.get_sync_state(&hash_key).await?.as_deref() == Some(h.as_str()) {
            report.skipped += 1;
            manifest.push(json!({
                "file": file_name,
                "name": name,
                "topic_key": tk,
            }));
            entries.push(ScanEntry {
                name,
                topic_key: tk,
                node_id: None,
                wikilinks: wikilink::extract(&parsed.body),
                changed: false,
            });
            continue;
        }

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

        let node_id = if let Some(existing) = node_id_by_topic_key(&shared, &tk).await {
            let uuid_part = existing.strip_prefix("memory:").unwrap_or(&existing);
            let u = Uuid::parse_str(uuid_part)
                .with_context(|| format!("bad node id {existing} for {tk}"))?;
            writer
                .save_as(u, &cm)
                .await
                .with_context(|| format!("upserting {}", path.display()))?;
            existing
        } else {
            let u = writer
                .save(&cm)
                .await
                .with_context(|| format!("saving {}", path.display()))?;
            format!("memory:{u}")
        };
        report.upserted += 1;
        engine.sqlite.set_sync_state(&hash_key, &h).await?;
        manifest.push(json!({
            "file": file_name,
            "name": name,
            "topic_key": tk,
        }));
        entries.push(ScanEntry {
            name,
            topic_key: tk,
            node_id: Some(node_id),
            wikilinks: wikilink::extract(&parsed.body),
            changed: true,
        });
    }

    report.edges_added = link_wikilinks(writer, &shared, &entries, &realm, &now).await?;
    report.archived = archive_deleted(
        engine,
        writer,
        &shared,
        &old_manifest,
        &manifest,
        &memory_dir,
        &now,
    )
    .await?;

    engine
        .sqlite
        .set_sync_state(&manifest_key, &Value::Array(manifest).to_string())
        .await?;
    Ok(report)
}

/// Append `RELATES_TO` edges for `[[wikilinks]]` between this project's
/// memories. Only links touching a file that changed this run are
/// (re-)emitted — unchanged files' edges already exist (deterministic ids).
async fn link_wikilinks(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    entries: &[ScanEntry],
    realm: &str,
    now: &str,
) -> Result<usize> {
    use std::collections::{HashMap, HashSet};

    let changed: HashSet<&str> = entries
        .iter()
        .filter(|e| e.changed)
        .map(|e| e.name.as_str())
        .collect();
    let by_name: HashMap<&str, &ScanEntry> = entries.iter().map(|e| (e.name.as_str(), e)).collect();

    // (src, dst) name pairs worth emitting this run.
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for e in entries {
        for target in &e.wikilinks {
            if target == &e.name || !by_name.contains_key(target.as_str()) {
                continue; // self-links and unresolved targets are dropped
            }
            if e.changed || changed.contains(target.as_str()) {
                pairs.push((e.name.as_str(), target.as_str()));
            }
        }
    }
    if pairs.is_empty() {
        return Ok(0);
    }

    // Resolve node ids (lazily for unchanged endpoints).
    let mut ids: HashMap<&str, String> = HashMap::new();
    for name in pairs.iter().flat_map(|(a, b)| [*a, *b]) {
        if ids.contains_key(name) {
            continue;
        }
        let entry = by_name[name];
        let id = match &entry.node_id {
            Some(id) => Some(id.clone()),
            None => node_id_by_topic_key(shared, &entry.topic_key).await,
        };
        if let Some(id) = id {
            ids.insert(name, id);
        }
    }

    let props = json!({"via": "cc-wikilink"});
    let rows: Vec<Value> = pairs
        .iter()
        .filter_map(|(src, dst)| match (ids.get(src), ids.get(dst)) {
            (Some(s), Some(d)) => Some(edge_row(
                s,
                d,
                kyma_memory::EDGE_RELATES_TO,
                realm,
                None,
                Some(&props),
                now,
            )),
            _ => None,
        })
        .collect();
    let n = rows.len();
    if n > 0 {
        writer.append_edge_rows(rows).await?;
    }
    Ok(n)
}

/// Archive nodes whose files disappeared: every topic key present in the
/// previous manifest but absent from this scan gets its node's latest
/// version re-appended with `status: archived` + `invalid_at`. Renames are
/// naturally exempt (same topic key, new file). The file's hash state is
/// reset so a reappearing file re-ingests (and thereby un-archives).
async fn archive_deleted(
    engine: &Engine,
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    old_manifest: &[Value],
    new_manifest: &[Value],
    memory_dir: &Path,
    now: &str,
) -> Result<usize> {
    use std::collections::HashSet;

    let live: HashSet<&str> = new_manifest
        .iter()
        .filter_map(|e| e.get("topic_key").and_then(Value::as_str))
        .collect();
    let mut archived = 0;
    for old in old_manifest {
        let Some(tk) = old.get("topic_key").and_then(Value::as_str) else {
            continue; // kyma-authored entries: writeback owns their lifecycle
        };
        if live.contains(tk) {
            continue;
        }
        if archive_node(writer, shared, tk, now).await? {
            archived += 1;
        }
        if let Some(f) = old.get("file").and_then(Value::as_str) {
            let key = format!("ccsync:hash:{}", memory_dir.join(f).display());
            engine.sqlite.set_sync_state(&key, "").await?;
        }
    }
    Ok(archived)
}

/// Append an archived version of the node carrying `topic_key`. Returns
/// false when there is nothing to do (no node, or already archived).
pub(crate) async fn archive_node(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    topic_key: &str,
    now: &str,
) -> Result<bool> {
    let q = format!(
        "WITH latest AS (SELECT *, \
           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn FROM {nt}) \
         SELECT {NODE_COLS} FROM latest WHERE rn = 1 AND topic_key = {tk} LIMIT 1",
        nt = kyma_memory::NODE_TABLE,
        tk = kyma_memory::sql::sql_str(topic_key),
    );
    let res = execute_sql(shared, kyma_memory::DEFAULT_DATABASE, &q, 1).await;
    let Some(mut row) = res
        .get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
    else {
        return Ok(false);
    };
    if row.get("status").and_then(Value::as_str) == Some("archived") {
        return Ok(false);
    }
    let mut prov = row
        .get("provenance")
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = prov.as_object_mut() {
        obj.insert("cc_archived_reason".into(), json!("file_deleted"));
        obj.insert("cc_archived_at".into(), json!(now));
    }
    if let Some(obj) = row.as_object_mut() {
        obj.insert("status".into(), json!("archived"));
        obj.insert("invalid_at".into(), json!(now));
        obj.insert("updated_at".into(), json!(now));
        obj.insert("provenance".into(), json!(prov.to_string()));
    }
    writer.append_node_rows(vec![row]).await?;
    Ok(true)
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
/// to the slug itself. Also returns the resolved project path when known.
fn resolve_realm(
    slug_dir: &Path,
    dir_slug: &str,
    known_paths: &[String],
) -> (String, Option<PathBuf>) {
    if let Some(p) = slug::resolve_project_path(dir_slug, known_paths) {
        return (slug::realm_for_path(&p), Some(p));
    }
    if let Some(cwd) = transcript_cwd(slug_dir) {
        return (slug::realm_for_path(&cwd), Some(cwd));
    }
    (dir_slug.to_string(), None)
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
pub(crate) async fn node_id_by_topic_key(
    shared: &SharedToolCtx,
    topic_key: &str,
) -> Option<String> {
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
