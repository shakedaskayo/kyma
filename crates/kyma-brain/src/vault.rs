//! Vault planning: memory rows → the deterministic set of files the
//! exporter commits.
//!
//! `plan_vault` is a pure function of (config, prior manifest, node rows,
//! edge rows): same inputs in any order produce byte-identical output. No
//! clocks, no randomness — the freshness stamp in `index.md` is the max
//! `updated_at` across included memories.

use std::collections::{BTreeMap, BTreeSet};

use crate::notes::{self, RelatedLink};
use crate::registry::{BrainConfig, VaultLayout};
use crate::types::{EdgeRow, Manifest, ManifestEntry, NoteRow, VaultFile};
use crate::{moc, BrainError, MANIFEST_PATH, WIKI_TOPIC_PREFIX};

/// Max Related links rendered per note.
const RELATED_CAP: usize = 8;

/// The planned vault: files to commit plus the new manifest (already
/// included in `files` as `.kyma/manifest.json`).
#[derive(Debug, Clone)]
pub struct VaultPlan {
    pub files: Vec<VaultFile>,
    pub manifest: Manifest,
    pub note_count: u64,
}

/// Folder name for a memory type (plural).
pub fn type_folder(memory_type: &str) -> &'static str {
    match memory_type {
        "decision" => "decisions",
        "preference" => "preferences",
        "learning" => "learnings",
        "summary" => "summaries",
        "procedure" => "procedures",
        "entity" => "entities",
        _ => "facts",
    }
}

/// Whether a node passes the brain's filters (realm selection is the
/// caller's job — it fetches only selected realms).
pub fn included(cfg: &BrainConfig, row: &NoteRow) -> bool {
    let f = &cfg.include;
    if !f.statuses.iter().any(|s| s == &row.status) {
        return false;
    }
    if let Some(types) = &f.memory_types {
        if !types.iter().any(|t| t == &row.memory_type) {
            return false;
        }
    }
    if let Some(min) = f.min_importance {
        if row.importance < min {
            return false;
        }
    }
    if !f.include_invalidated && row.invalid_at.is_some() {
        return false;
    }
    true
}

/// The wiki page slug when a node is a gardener wiki page for this brain.
fn wiki_slug(cfg: &BrainConfig, row: &NoteRow) -> Option<String> {
    let tk = row.topic_key.as_deref()?;
    let rest = tk.strip_prefix(WIKI_TOPIC_PREFIX)?;
    let (brain, slug) = rest.split_once(':')?;
    if brain != cfg.name || slug.is_empty() {
        return None;
    }
    Some(notes::title_slug(slug))
}

/// Mint (or keep) the repo path of a note. Paths are minted once: a memory
/// that already has a path in the prior manifest keeps it verbatim, even if
/// its title, type, or realm changed since (frontmatter is truth; the
/// filename is an address).
fn note_path(cfg: &BrainConfig, row: &NoteRow, prior: &BTreeMap<String, String>) -> String {
    if let Some(p) = prior.get(&row.id) {
        return p.clone();
    }
    if let Some(slug) = wiki_slug(cfg, row) {
        return format!("wiki/{slug}.md");
    }
    let stem = notes::note_stem(&row.title, &row.id);
    if row.memory_type == "entity" {
        return format!("entities/{stem}.md");
    }
    let folder = type_folder(&row.memory_type);
    match cfg.layout {
        VaultLayout::Flat => format!("notes/{folder}/{stem}.md"),
        VaultLayout::ByRealm => {
            let realm = sanitize_realm_segment(&row.realm);
            format!("notes/{realm}/{folder}/{stem}.md")
        }
    }
}

fn sanitize_realm_segment(realm: &str) -> String {
    let cleaned: String = realm
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' })
        .collect();
    let cleaned = cleaned.trim_matches(['.', '-']).to_string();
    if cleaned.is_empty() { "default".to_string() } else { cleaned }
}

fn validate_path(path: &str) -> Result<(), BrainError> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..");
    if bad {
        return Err(BrainError::InvalidPath(path.to_string()));
    }
    Ok(())
}

/// Plan the full vault. `prior_manifest` carries filename stability across
/// exports; pass `Manifest::default()` on first run.
pub fn plan_vault(
    cfg: &BrainConfig,
    prior_manifest: &Manifest,
    nodes: &[NoteRow],
    edges: &[EdgeRow],
) -> Result<VaultPlan, BrainError> {
    let prior_paths = prior_manifest.paths_by_memory_id();

    // Deterministic node order: by id (dedup keeps first occurrence).
    let mut by_id: BTreeMap<String, &NoteRow> = BTreeMap::new();
    for n in nodes {
        if included(cfg, n) {
            by_id.entry(n.id.clone()).or_insert(n);
        }
    }

    // Path assignment; collisions (two new notes with identical stems can't
    // happen thanks to the id suffix, but a prior path could equal a newly
    // minted one) resolved deterministically by id order — later ids get a
    // numeric suffix.
    let mut paths: BTreeMap<String, String> = BTreeMap::new(); // id -> path
    let mut taken: BTreeSet<String> = BTreeSet::new();
    for (id, row) in &by_id {
        let mut path = note_path(cfg, row, &prior_paths);
        validate_path(&path)?;
        let mut n = 1;
        while taken.contains(&path) {
            n += 1;
            path = path.replace(".md", &format!("-{n}.md"));
        }
        taken.insert(path.clone());
        paths.insert(id.clone(), path);
    }

    // Related links from edges among included nodes.
    let node_key = |id: &str| format!("memory:{id}");
    let bare = |node_id: &str| node_id.strip_prefix("memory:").unwrap_or(node_id).to_string();
    let mut related: BTreeMap<String, Vec<RelatedLink>> = BTreeMap::new();
    let mut seen_edges: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut sorted_edges: Vec<&EdgeRow> = edges.iter().collect();
    sorted_edges.sort_by(|a, b| (&a.src, &a.dst, &a.edge_type).cmp(&(&b.src, &b.dst, &b.edge_type)));
    for e in sorted_edges {
        let (src, dst) = (bare(&e.src), bare(&e.dst));
        for (from, to) in [(&src, &dst), (&dst, &src)] {
            if !by_id.contains_key(from) || !by_id.contains_key(to) || from == to {
                continue;
            }
            if !seen_edges.insert((from.clone(), to.clone(), e.edge_type.clone())) {
                continue;
            }
            let target_path = &paths[to];
            related.entry(from.clone()).or_default().push(RelatedLink {
                target: target_path.trim_end_matches(".md").to_string(),
                title: by_id[to].title.clone(),
                edge_type: e.edge_type.clone(),
            });
        }
    }
    let _ = node_key; // kept for clarity of the id convention

    let mut files: Vec<VaultFile> = Vec::with_capacity(by_id.len() + 8);
    let mut entries: Vec<ManifestEntry> = Vec::new();

    for (id, row) in &by_id {
        let path = paths[id].clone();
        let mut rel = related.remove(id).unwrap_or_default();
        rel.sort_by(|a, b| (&a.edge_type, &a.target).cmp(&(&b.edge_type, &b.target)));
        rel.dedup_by(|a, b| a.target == b.target && a.edge_type == b.edge_type);
        rel.truncate(RELATED_CAP);
        let text = notes::render_note(row, &rel, cfg.include.redact_private_spans);
        files.push(VaultFile { path: path.clone(), bytes: text.into_bytes() });
        entries.push(ManifestEntry { path, memory_id: Some(id.clone()) });
    }

    // Generated top-level files.
    let included_rows: Vec<&NoteRow> = by_id.values().copied().collect();
    for (path, text) in [
        ("README.md".to_string(), moc::render_readme(cfg)),
        ("CONTRIBUTING.md".to_string(), moc::render_contributing(cfg)),
        ("index.md".to_string(), moc::render_index(cfg, &included_rows, &paths)),
    ] {
        files.push(VaultFile { path: path.clone(), bytes: text.into_bytes() });
        entries.push(ManifestEntry { path, memory_id: None });
    }
    for (realm, text) in moc::render_realm_indexes(cfg, &included_rows, &paths) {
        let path = format!("notes/{}/index.md", sanitize_realm_segment(&realm));
        files.push(VaultFile { path: path.clone(), bytes: text.into_bytes() });
        entries.push(ManifestEntry { path, memory_id: None });
    }

    entries.push(ManifestEntry { path: MANIFEST_PATH.to_string(), memory_id: None });
    let manifest = Manifest { version: Manifest::CURRENT_VERSION, entries };
    files.push(VaultFile { path: MANIFEST_PATH.to_string(), bytes: manifest.to_bytes() });

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let note_count = by_id.len() as u64;
    Ok(VaultPlan { files, manifest, note_count })
}

/// Files seeded exactly once at brain creation (first commit) and never
/// touched by the exporter afterwards — users may tweak them and their
/// edits survive because these paths are absent from the manifest.
pub fn seed_files(cfg: &BrainConfig) -> Vec<VaultFile> {
    let graph_json = r#"{
  "colorGroups": [
    { "query": "path:notes/decisions", "color": { "a": 1, "rgb": 14701138 } },
    { "query": "path:notes/learnings", "color": { "a": 1, "rgb": 4886754 } },
    { "query": "path:notes/procedures", "color": { "a": 1, "rgb": 5431473 } },
    { "query": "path:notes/facts", "color": { "a": 1, "rgb": 11621088 } },
    { "query": "path:notes/preferences", "color": { "a": 1, "rgb": 15558949 } },
    { "query": "path:entities", "color": { "a": 1, "rgb": 2201331 } },
    { "query": "path:wiki", "color": { "a": 1, "rgb": 16766720 } }
  ]
}
"#;
    let app_json = "{\n  \"newFileLocation\": \"folder\",\n  \"newFileFolderPath\": \"inbox\"\n}\n";
    let gitignore = ".obsidian/workspace*.json\n.obsidian/cache\n.trash/\n.DS_Store\n";
    let inbox_keep = format!(
        "Drop new notes here (see CONTRIBUTING.md). They are ingested into kyma's `{}` brain and re-filed under notes/ on the next export.\n",
        cfg.name
    );
    vec![
        VaultFile { path: ".obsidian/app.json".into(), bytes: app_json.as_bytes().to_vec() },
        VaultFile { path: ".obsidian/graph.json".into(), bytes: graph_json.as_bytes().to_vec() },
        VaultFile { path: ".gitignore".into(), bytes: gitignore.as_bytes().to_vec() },
        VaultFile { path: "inbox/README.md".into(), bytes: inbox_keep.into_bytes() },
    ]
}
