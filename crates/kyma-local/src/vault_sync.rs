//! Sync a local Obsidian vault into the memory store. Files are the source
//! of truth: notes upsert as memory nodes by topic key, wikilinks become
//! `RELATES_TO` edges, deleted notes archive (and un-archive when the file
//! reappears). Modeled on [`crate::cc_sync`] but vault-flavored:
//!
//! - **Frontmatter is optional** — plain markdown ingests as-is; a YAML
//!   block (when present) contributes `tags` and a `title`.
//! - **The walk is recursive**, skipping dot-directories (`.obsidian/`,
//!   `.trash/`, …).
//! - **Link syntax is Obsidian's** — `[[Target|alias]]`, `[[Target#h]]`,
//!   `![[embed]]` all normalize to the target note, resolved by file stem,
//!   case-insensitively.
//!
//! Idempotency mirrors cc-sync: normalized content hashes in `sync_state`
//! (`obsidian:hash:<source_id>:<rel_path>`) skip unchanged files without
//! re-embedding; a per-source manifest (`obsidian:manifest:<source_id>`)
//! drives archive-on-delete.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::cc_sync::{archive_node, node_id_by_topic_key};
use crate::Engine;
use kyma_ccmem::{hash, wikilink};
use kyma_memory::{rows::edge_row, CreateMemory, MemoryType, MemoryWriter};
use kyma_server::agent::SharedToolCtx;

/// Importance assigned to vault notes: deliberately curated knowledge, but
/// not an explicit "remember this" save (0.7) — between that and the 0.5
/// default.
const NOTE_IMPORTANCE: f32 = 0.6;

/// One vault to sync. Resolved from the data source row by the watcher
/// manager; injectable for tests.
pub(crate) struct VaultSyncOptions {
    /// `data_sources` row id — namespaces topic keys + sync_state.
    pub source_id: String,
    /// Expanded absolute vault root.
    pub vault_path: PathBuf,
    /// Realm the notes land in (`vault_name` config or folder basename).
    pub realm: String,
}

/// Outcome of one full vault pass.
#[derive(Debug, Default)]
pub(crate) struct VaultSyncReport {
    /// `.md` files found (outside dot-directories).
    pub seen: usize,
    /// Notes ingested or re-ingested (created or new version).
    pub upserted: usize,
    /// Notes unchanged since the last pass (hash hit).
    pub skipped: usize,
    /// `RELATES_TO` edges appended for resolved wikilinks.
    pub edges_added: usize,
    /// Nodes archived because their file disappeared.
    pub archived: usize,
    /// Unreadable files (logged and skipped).
    pub errors: usize,
}

impl VaultSyncReport {
    /// Rollup in the watcher `last_scan` shape
    /// (`{seen, processed, errors, duration_ms, at, detail}`).
    pub(crate) fn last_scan_value(
        &self,
        duration_ms: u64,
        at: chrono::DateTime<chrono::Utc>,
        realm: &str,
    ) -> Value {
        json!({
            "seen": self.seen,
            "processed": self.upserted,
            "errors": self.errors,
            "duration_ms": duration_ms,
            "at": at.to_rfc3339(),
            "detail": {
                "realm": realm,
                "skipped": self.skipped,
                "edges_added": self.edges_added,
                "archived": self.archived,
            },
        })
    }
}

/// What one scanned note contributed — input to the wikilink edge pass and
/// the deletion manifest.
struct ScanEntry {
    /// Note name (file stem) — the wikilink resolution key.
    name: String,
    topic_key: String,
    /// Set when this run wrote the node; looked up lazily otherwise.
    node_id: Option<String>,
    wikilinks: Vec<String>,
    changed: bool,
}

/// Lenient Obsidian frontmatter: optional `---` fenced YAML block with
/// `tags` (string or sequence) and `title`. Returns `(title, tags, body)`.
/// No frontmatter, or a malformed block, yields the whole file as body.
fn parse_note(raw: &str) -> (Option<String>, Vec<String>, String) {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return (None, Vec::new(), raw.to_string());
    };
    let (yaml, body) = if let Some(i) = rest.find("\n---\n") {
        (&rest[..i], &rest[i + 5..])
    } else if let Some(i) = rest.find("\n---") {
        if !rest[i + 4..].trim().is_empty() {
            return (None, Vec::new(), raw.to_string());
        }
        (&rest[..i], "")
    } else {
        return (None, Vec::new(), raw.to_string());
    };
    let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(yaml) else {
        return (None, Vec::new(), raw.to_string());
    };
    let title = val
        .get("title")
        .and_then(serde_yaml::Value::as_str)
        .map(str::to_string);
    let tags = match val.get("tags") {
        Some(serde_yaml::Value::String(s)) => s
            .split([',', ' '])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(serde_yaml::Value::as_str)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    (title, tags, body.trim_start_matches('\n').to_string())
}

/// All `.md` files under `root`, skipping dot-directories. Relative paths
/// use `/` separators; sorted for determinism.
fn walk_vault(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    // The root read must succeed (a missing vault is a real error the run
    // reports); nested read failures are skipped like unreadable files.
    if !root.is_dir() {
        anyhow::bail!("vault path {} is not a directory", root.display());
    }
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !name.starts_with('.') {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push((path, rel));
        }
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(out)
}

/// Sync one vault: scan, upsert changed notes, link wikilinks, archive
/// deletions, persist the manifest.
pub(crate) async fn run_once(
    engine: &Engine,
    writer: &MemoryWriter,
    opts: &VaultSyncOptions,
) -> Result<VaultSyncReport> {
    let shared = SharedToolCtx {
        federation: None,
        catalog: engine.catalog.clone(),
        format: engine.format.clone(),
        pool: None,
        memory: None,
        hitl: None,
    };
    let mut report = VaultSyncReport::default();
    let files = walk_vault(&opts.vault_path)?;

    let manifest_key = format!("obsidian:manifest:{}", opts.source_id);
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

    for (path, rel) in files {
        report.seen += 1;
        let Ok(raw) = std::fs::read_to_string(&path) else {
            tracing::warn!(file = %path.display(), "vault-sync: unreadable file, skipping");
            report.errors += 1;
            continue;
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let (title, fm_tags, body) = parse_note(&raw);
        let h = hash::content_hash(&name, None, &body);
        let hash_key = format!("obsidian:hash:{}:{}", opts.source_id, rel);
        let tk = format!("obsidian:{}/{}", opts.source_id, rel);

        if engine.sqlite.get_sync_state(&hash_key).await?.as_deref() == Some(h.as_str()) {
            report.skipped += 1;
            manifest.push(json!({ "rel_path": &rel, "topic_key": &tk }));
            entries.push(ScanEntry {
                name,
                topic_key: tk,
                node_id: None,
                wikilinks: wikilink::extract_normalized(&body),
                changed: false,
            });
            continue;
        }

        let mut tags = vec!["obsidian".to_string()];
        tags.extend(fm_tags.into_iter().filter(|t| t != "obsidian"));
        let mut cm = CreateMemory::new(body.clone());
        cm.title = Some(title.unwrap_or_else(|| name.clone()));
        cm.memory_type = MemoryType::Fact;
        cm.tags = tags;
        cm.realm = opts.realm.clone();
        cm.importance = NOTE_IMPORTANCE;
        cm.topic_key = Some(tk.clone());
        cm.provenance = Some(json!({
            "source": "obsidian",
            "source_id": opts.source_id,
            "vault_path": opts.vault_path.display().to_string(),
            "rel_path": &rel,
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
        manifest.push(json!({ "rel_path": &rel, "topic_key": &tk }));
        entries.push(ScanEntry {
            name,
            topic_key: tk,
            node_id: Some(node_id),
            wikilinks: wikilink::extract_normalized(&body),
            changed: true,
        });
    }

    report.edges_added = link_wikilinks(writer, &shared, &entries, &opts.realm, &now).await?;
    report.archived = archive_deleted(
        engine,
        writer,
        &shared,
        &old_manifest,
        &manifest,
        &opts.source_id,
        &now,
    )
    .await?;

    engine
        .sqlite
        .set_sync_state(&manifest_key, &Value::Array(manifest).to_string())
        .await?;
    Ok(report)
}

/// Append `RELATES_TO` edges for resolved wikilinks. Resolution is by note
/// name (file stem), case-insensitive — Obsidian's own semantics. Only
/// links touching a note that changed this run are (re-)emitted: unchanged
/// notes' edges already exist (deterministic edge ids).
async fn link_wikilinks(
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    entries: &[ScanEntry],
    realm: &str,
    now: &str,
) -> Result<usize> {
    use std::collections::HashMap;

    let by_name: HashMap<String, &ScanEntry> = entries
        .iter()
        .map(|e| (e.name.to_lowercase(), e))
        .collect();

    // (src, dst) lowercase-name pairs worth emitting this run.
    let mut pairs: Vec<(String, String)> = Vec::new();
    for e in entries {
        let src_key = e.name.to_lowercase();
        for target in &e.wikilinks {
            let dst_key = target.to_lowercase();
            if dst_key == src_key {
                continue; // self-link
            }
            let Some(dst) = by_name.get(&dst_key) else {
                continue; // unresolved target
            };
            if e.changed || dst.changed {
                pairs.push((src_key.clone(), dst_key));
            }
        }
    }
    if pairs.is_empty() {
        return Ok(0);
    }

    // Resolve node ids (lazily for unchanged endpoints).
    let mut ids: HashMap<String, String> = HashMap::new();
    for name in pairs.iter().flat_map(|(a, b)| [a.clone(), b.clone()]) {
        if ids.contains_key(&name) {
            continue;
        }
        let entry = by_name[&name];
        let id = match &entry.node_id {
            Some(id) => Some(id.clone()),
            None => node_id_by_topic_key(shared, &entry.topic_key).await,
        };
        if let Some(id) = id {
            ids.insert(name, id);
        }
    }

    let props = json!({ "via": "obsidian-wikilink" });
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

/// Archive nodes whose files disappeared: topic keys in the previous
/// manifest but absent from this scan get an archived version appended. The
/// file's hash state is reset so a reappearing file re-ingests (and thereby
/// un-archives).
async fn archive_deleted(
    engine: &Engine,
    writer: &MemoryWriter,
    shared: &SharedToolCtx,
    old_manifest: &[Value],
    new_manifest: &[Value],
    source_id: &str,
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
            continue;
        };
        if live.contains(tk) {
            continue;
        }
        if archive_node(writer, shared, tk, now).await? {
            archived += 1;
        }
        if let Some(rel) = old.get("rel_path").and_then(Value::as_str) {
            let key = format!("obsidian:hash:{source_id}:{rel}");
            engine.sqlite.set_sync_state(&key, "").await?;
        }
    }
    Ok(archived)
}
