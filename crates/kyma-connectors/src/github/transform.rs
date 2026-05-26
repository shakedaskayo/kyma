//! Transform GitHub REST API payloads into the graph node/edge schema.
//!
//! ## Schema (CAP-safe: ≤5 columns per row)
//!
//! **github_nodes** rows:
//!   `{ "id": String, "labels": String, "name": String, "props": String }`
//!   (4 columns)
//!
//! **github_edges** rows:
//!   `{ "id": String, "src": String, "dst": String, "type": String, "props": String }`
//!   (5 columns)
//!
//! `labels` is a **single string** (e.g. `"Repository"`) — not an array —
//! so it lands cleanly in a Utf8 column and `parse_labels` can wrap it.
//! `props` is a JSON blob so entity-specific fields don't cause column
//! explosion.
//!
//! ## Code graph (B2)
//! `code_to_graph` appends `Directory`/`CodeFile` nodes and `CONTAINS`/`IMPORTS`
//! edges derived from the git-tree + per-file import lists produced by
//! `parse::extract_imports`.

use crate::types::{GraphHint, TableRows};
use serde_json::{json, Map, Value};

pub const NODE_TABLE: &str = "github_nodes";
pub const EDGE_TABLE: &str = "github_edges";
pub const GRAPH_NAME: &str = "github";

/// Raw records produced by the GitHub client before graph transform.
#[derive(Debug, Clone)]
pub enum RawRecord {
    Repo(Value),
    User(Value),
    Branch { owner: String, name: String, branch: Value },
    Pull { owner: String, repo: String, pull: Value },
    Issue { owner: String, repo: String, issue: Value },
}

// ── id helpers ───────────────────────────────────────────────────────────────

pub fn repo_id(owner: &str, name: &str) -> String {
    format!("repo:{owner}/{name}")
}

pub fn user_id(login: &str) -> String {
    format!("user:{login}")
}

pub fn branch_id(owner: &str, repo: &str, branch: &str) -> String {
    format!("branch:{owner}/{repo}@{branch}")
}

pub fn pr_id(owner: &str, repo: &str, number: i64) -> String {
    format!("pr:{owner}/{repo}#{number}")
}

pub fn issue_id(owner: &str, repo: &str, number: i64) -> String {
    format!("issue:{owner}/{repo}#{number}")
}

pub fn dir_id(owner: &str, repo: &str, path: &str) -> String {
    format!("dir:{owner}/{repo}:{path}")
}

pub fn file_id(owner: &str, repo: &str, path: &str) -> String {
    format!("file:{owner}/{repo}:{path}")
}

pub fn module_id(raw: &str) -> String {
    format!("module:{raw}")
}

/// Deterministic id for a code symbol (function/class). `kind` is `func` or
/// `class`; `line` disambiguates same-named symbols in one file.
pub fn symbol_id(owner: &str, repo: &str, path: &str, name: &str, line: usize, kind: &str) -> String {
    format!("{kind}:{owner}/{repo}:{path}#{name}#{line}")
}

/// Deterministic edge id: `src|TYPE|dst`.
pub fn edge_id(src: &str, rel: &str, dst: &str) -> String {
    format!("{src}|{rel}|{dst}")
}

/// Node labels re-fetched in full on every poll (vs the cursor-incremental
/// pulls/issues stream). Their rows are gated by [`refetch_signature`].
pub const REFETCH_NODE_LABELS: [&str; 3] = ["Repository", "User", "Branch"];
/// Edge types re-fetched in full on every poll.
pub const REFETCH_EDGE_TYPES: [&str; 2] = ["OWNS", "HAS_BRANCH"];

/// True if this node row comes from a full-refetch module.
pub fn is_refetch_node(n: &Value) -> bool {
    n.get("labels").and_then(Value::as_str).is_some_and(|l| REFETCH_NODE_LABELS.contains(&l))
}
/// True if this edge row comes from a full-refetch module.
pub fn is_refetch_edge(e: &Value) -> bool {
    e.get("type").and_then(Value::as_str).is_some_and(|t| REFETCH_EDGE_TYPES.contains(&t))
}

/// A stable, order-independent signature over the full-refetch metadata rows.
///
/// Used to skip re-emitting unchanged repo/user/branch rows every tick: the
/// connector re-fetches these modules in full each poll, and the node/edge
/// tables are append-only, so without a gate a continuous connector grows the
/// tables without bound. Hashing the *emitted rows* (not the raw API payload)
/// means only changes to the fields we actually store trigger a re-emit.
pub fn refetch_signature(nodes: &[Value], edges: &[Value]) -> String {
    use std::hash::{Hash, Hasher};
    let mut parts: Vec<String> = Vec::new();
    for n in nodes.iter().filter(|n| is_refetch_node(n)) {
        parts.push(serde_json::to_string(n).unwrap_or_default());
    }
    for e in edges.iter().filter(|e| is_refetch_edge(e)) {
        parts.push(serde_json::to_string(e).unwrap_or_default());
    }
    parts.sort();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in &parts {
        p.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

// ── node builders ─────────────────────────────────────────────────────────────

fn make_node(id: &str, label: &str, name: &str, extra: Map<String, Value>) -> Value {
    json!({
        "id": id,
        "labels": label,
        "name": name,
        "props": serde_json::to_string(&extra).unwrap_or_default(),
    })
}

fn make_edge(src: &str, rel: &str, dst: &str, extra: Map<String, Value>) -> Value {
    let id = edge_id(src, rel, dst);
    json!({
        "id": id,
        "src": src,
        "dst": dst,
        "type": rel,
        "props": serde_json::to_string(&extra).unwrap_or_default(),
    })
}

// ── `closes #N` parser ───────────────────────────────────────────────────────

/// Parse issue numbers mentioned in a PR body via `closes/fixes/resolves #N`.
fn parse_closes(body: &str) -> Vec<i64> {
    let re_pat = ["closes", "fixes", "resolves"];
    let mut numbers = Vec::new();
    let lower = body.to_ascii_lowercase();
    for line in lower.lines() {
        for kw in &re_pat {
            if let Some(pos) = line.find(kw) {
                let rest = &line[pos + kw.len()..].trim_start();
                if let Some(rest) = rest.strip_prefix('#') {
                    let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num_str.parse::<i64>() {
                        numbers.push(n);
                    }
                }
            }
        }
    }
    numbers.dedup();
    numbers
}

// ── main metadata transform ──────────────────────────────────────────────────

/// Convert a slice of `RawRecord`s into `(Vec<TableRows>, GraphHint)`.
///
/// The two `TableRows` are for `github_nodes` and `github_edges` respectively.
/// Returns a deterministic result: same input → same node/edge ids.
pub fn to_graph(records: &[RawRecord]) -> (Vec<TableRows>, GraphHint) {
    let mut nodes: Vec<Value> = Vec::new();
    let mut edges: Vec<Value> = Vec::new();

    for record in records {
        match record {
            RawRecord::Repo(repo) => {
                let owner = repo["owner"]["login"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let repo_name = repo["name"].as_str().unwrap_or("").to_string();
                let full_name = repo["full_name"]
                    .as_str()
                    .unwrap_or(&format!("{owner}/{repo_name}"))
                    .to_string();
                let rid = repo_id(&owner, &repo_name);

                let mut props = Map::new();
                for key in &[
                    "description", "html_url", "default_branch", "private",
                    "fork", "stargazers_count", "open_issues_count",
                    "language", "pushed_at", "updated_at", "created_at",
                ] {
                    if let Some(v) = repo.get(*key) {
                        props.insert(key.to_string(), v.clone());
                    }
                }
                nodes.push(make_node(&rid, "Repository", &full_name, props));

                // OWNS edge: user → repo
                if !owner.is_empty() {
                    let uid = user_id(&owner);
                    let mut uprops = Map::new();
                    uprops.insert("login".into(), Value::String(owner.clone()));
                    // Upsert owner User node (minimal info — will be enriched
                    // by a dedicated User record if contributors are fetched)
                    nodes.push(make_node(&uid, "User", &owner, uprops.clone()));
                    edges.push(make_edge(&uid, "OWNS", &rid, Map::new()));
                }
            }

            RawRecord::User(user) => {
                let login = user["login"].as_str().unwrap_or("").to_string();
                let uid = user_id(&login);
                let mut props = Map::new();
                for key in &["html_url", "avatar_url", "type", "site_admin"] {
                    if let Some(v) = user.get(*key) {
                        props.insert(key.to_string(), v.clone());
                    }
                }
                nodes.push(make_node(&uid, "User", &login, props));
            }

            RawRecord::Branch { owner, name: repo_name, branch } => {
                let branch_name = branch["name"].as_str().unwrap_or("").to_string();
                let bid = branch_id(owner, repo_name, &branch_name);
                let rid = repo_id(owner, repo_name);

                let mut props = Map::new();
                if let Some(sha) = branch["commit"]["sha"].as_str() {
                    props.insert("sha".into(), Value::String(sha.to_string()));
                }
                if let Some(protected) = branch.get("protected") {
                    props.insert("protected".into(), protected.clone());
                }
                nodes.push(make_node(&bid, "Branch", &branch_name, props));
                edges.push(make_edge(&rid, "HAS_BRANCH", &bid, Map::new()));
            }

            RawRecord::Pull { owner, repo: repo_name, pull } => {
                let number = pull["number"].as_i64().unwrap_or(0);
                let pid = pr_id(owner, repo_name, number);
                let rid = repo_id(owner, repo_name);
                let title = pull["title"].as_str().unwrap_or("").to_string();

                let mut props = Map::new();
                for key in &[
                    "html_url", "state", "draft", "created_at", "updated_at",
                    "closed_at", "merged_at", "body",
                ] {
                    if let Some(v) = pull.get(*key) {
                        props.insert(key.to_string(), v.clone());
                    }
                }
                nodes.push(make_node(&pid, "PullRequest", &title, props));
                edges.push(make_edge(&rid, "HAS_PULL_REQUEST", &pid, Map::new()));

                // AUTHORED edge: author → pr
                if let Some(login) = pull["user"]["login"].as_str() {
                    let uid = user_id(login);
                    edges.push(make_edge(&uid, "AUTHORED", &pid, Map::new()));
                }

                // ON_BRANCH: pr → head branch
                if let Some(head_ref) = pull["head"]["ref"].as_str() {
                    let bid = branch_id(owner, repo_name, head_ref);
                    edges.push(make_edge(&pid, "ON_BRANCH", &bid, Map::new()));
                }

                // TARGETS_BRANCH: pr → base branch
                if let Some(base_ref) = pull["base"]["ref"].as_str() {
                    let bid = branch_id(owner, repo_name, base_ref);
                    edges.push(make_edge(&pid, "TARGETS_BRANCH", &bid, Map::new()));
                }

                // CLOSES_ISSUE: parse body
                if let Some(body) = pull["body"].as_str() {
                    for issue_num in parse_closes(body) {
                        let iid = issue_id(owner, repo_name, issue_num);
                        edges.push(make_edge(&pid, "CLOSES_ISSUE", &iid, Map::new()));
                    }
                }
            }

            RawRecord::Issue { owner, repo: repo_name, issue } => {
                // GitHub issues API returns PR stubs as issues; skip them.
                if issue.get("pull_request").is_some() {
                    continue;
                }
                let number = issue["number"].as_i64().unwrap_or(0);
                let iid = issue_id(owner, repo_name, number);
                let rid = repo_id(owner, repo_name);
                let title = issue["title"].as_str().unwrap_or("").to_string();

                let mut props = Map::new();
                for key in &[
                    "html_url", "state", "created_at", "updated_at",
                    "closed_at", "body",
                ] {
                    if let Some(v) = issue.get(*key) {
                        props.insert(key.to_string(), v.clone());
                    }
                }
                // Labels as JSON array in props
                if let Some(labels) = issue["labels"].as_array() {
                    let label_names: Vec<_> = labels
                        .iter()
                        .filter_map(|l| l["name"].as_str().map(|s| Value::String(s.to_string())))
                        .collect();
                    props.insert("github_labels".into(), Value::Array(label_names));
                }
                nodes.push(make_node(&iid, "Issue", &title, props));
                edges.push(make_edge(&rid, "HAS_ISSUE", &iid, Map::new()));

                // AUTHORED edge: author → issue
                if let Some(login) = issue["user"]["login"].as_str() {
                    let uid = user_id(login);
                    edges.push(make_edge(&uid, "AUTHORED", &iid, Map::new()));
                }
            }
        }
    }

    build_tables(nodes, edges)
}

// ── Code graph transform (B2) ─────────────────────────────────────────────────

/// A file path + its extracted imports and symbol definitions.
pub struct FileImports {
    /// Repo-relative path (e.g. `"src/main.rs"`).
    pub path: String,
    /// File size in bytes (from the tree entry).
    pub size: usize,
    /// Detected language name (e.g. `"rust"`).
    pub language: String,
    /// Import targets extracted by tree-sitter.
    pub imports: Vec<ImportTarget>,
    /// Function/class definitions extracted by tree-sitter.
    pub symbols: Vec<crate::github::parse::CodeSymbol>,
}

/// A single resolved/unresolved import from a source file.
pub struct ImportTarget {
    /// The raw import string as it appears in source.
    pub raw: String,
    /// 1-based source line.
    pub line: usize,
    /// Path in the same repo that this import resolves to, if resolvable.
    pub resolved_path: Option<String>,
}

/// Build the code portion of the graph from a file tree + parsed imports.
///
/// Emits:
/// - `Directory` nodes for every directory that appears in the tree.
/// - `CodeFile` nodes for every file.
/// - `CONTAINS` edges: `repo → dir/file` (top-level) and `dir → child`.
/// - `IMPORTS` edges: `file → file` (resolved) or `file → external module`
///   (unresolved).
///
/// Results are appended to the existing `nodes`/`edges` vecs (callers
/// typically call `to_graph` first for metadata and then this function).
pub fn code_to_graph(
    owner: &str,
    repo_name: &str,
    files: &[FileImports],
    existing_nodes: Vec<Value>,
    existing_edges: Vec<Value>,
) -> (Vec<TableRows>, GraphHint) {
    let mut nodes = existing_nodes;
    let mut edges = existing_edges;

    let rid = repo_id(owner, repo_name);

    // Collect all directories that appear in the tree.
    let mut dirs: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for fi in files {
        // All ancestor directories.
        let mut p = std::path::Path::new(&fi.path);
        while let Some(parent) = p.parent() {
            let ps = parent.to_string_lossy().to_string();
            if ps.is_empty() || ps == "." {
                break;
            }
            dirs.insert(ps);
            p = parent;
        }
    }

    // Emit Directory nodes.
    for dir_path in &dirs {
        let did = dir_id(owner, repo_name, dir_path);
        let name = std::path::Path::new(dir_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dir_path.clone());
        nodes.push(make_node(&did, "Directory", &name, Map::new()));
    }

    // Emit CodeFile nodes.
    for fi in files {
        let fid = file_id(owner, repo_name, &fi.path);
        let name = std::path::Path::new(&fi.path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| fi.path.clone());
        let loc = 0usize; // LOC counting requires content; not available here
        let mut props = Map::new();
        props.insert("language".into(), Value::String(fi.language.clone()));
        props.insert("size".into(), Value::Number(fi.size.into()));
        props.insert("loc".into(), Value::Number(loc.into()));
        nodes.push(make_node(&fid, "CodeFile", &name, props));
    }

    // Emit CONTAINS edges.
    // Helper: parent id (repo or dir) for a given path.
    let parent_node_id = |path: &str| -> String {
        let p = std::path::Path::new(path);
        match p.parent() {
            Some(parent) => {
                let ps = parent.to_string_lossy().to_string();
                if ps.is_empty() || ps == "." {
                    rid.clone()
                } else {
                    dir_id(owner, repo_name, &ps)
                }
            }
            None => rid.clone(),
        }
    };

    for dir_path in &dirs {
        let did = dir_id(owner, repo_name, dir_path);
        let pid = parent_node_id(dir_path);
        edges.push(make_edge(&pid, "CONTAINS", &did, Map::new()));
    }

    for fi in files {
        let fid = file_id(owner, repo_name, &fi.path);
        let pid = parent_node_id(&fi.path);
        edges.push(make_edge(&pid, "CONTAINS", &fid, Map::new()));
    }

    // Emit IMPORTS edges.
    for fi in files {
        let fid = file_id(owner, repo_name, &fi.path);
        for imp in &fi.imports {
            let (dst_id, resolved) = match &imp.resolved_path {
                Some(rp) => (file_id(owner, repo_name, rp), true),
                None => (module_id(&imp.raw), false),
            };

            // Emit external module node if not resolved (best-effort dedup via
            // the BTreeMap pass below).
            if !resolved {
                nodes.push(make_node(&dst_id, "Module", &imp.raw, Map::new()));
            }

            let mut eprops = Map::new();
            eprops.insert("raw".into(), Value::String(imp.raw.clone()));
            eprops.insert("line".into(), Value::Number(imp.line.into()));
            eprops.insert("resolved".into(), Value::Bool(resolved));
            edges.push(make_edge(&fid, "IMPORTS", &dst_id, eprops));
        }
    }

    // Emit CodeFunction / CodeClass nodes + DEFINES edges (file → symbol).
    for fi in files {
        let fid = file_id(owner, repo_name, &fi.path);
        for sym in &fi.symbols {
            let (label, kind) = match sym.kind {
                crate::github::parse::SymbolKind::Function => ("CodeFunction", "func"),
                crate::github::parse::SymbolKind::Class => ("CodeClass", "class"),
            };
            let sid = symbol_id(owner, repo_name, &fi.path, &sym.name, sym.line, kind);
            let mut props = Map::new();
            props.insert("language".into(), Value::String(fi.language.clone()));
            props.insert("line".into(), Value::Number(sym.line.into()));
            props.insert("file".into(), Value::String(fi.path.clone()));
            nodes.push(make_node(&sid, label, &sym.name, props));
            edges.push(make_edge(&fid, "DEFINES", &sid, Map::new()));
        }
    }

    build_tables(nodes, edges)
}

// ── shared dedup + table builder ─────────────────────────────────────────────

fn build_tables(nodes: Vec<Value>, edges: Vec<Value>) -> (Vec<TableRows>, GraphHint) {
    // Deduplicate nodes by id (last-writer wins).
    let mut node_map: std::collections::BTreeMap<String, Value> =
        std::collections::BTreeMap::new();
    for n in nodes {
        if let Some(id) = n["id"].as_str() {
            node_map.insert(id.to_string(), n);
        }
    }

    // Deduplicate edges by id.
    let mut edge_map: std::collections::BTreeMap<String, Value> =
        std::collections::BTreeMap::new();
    for e in edges {
        if let Some(id) = e["id"].as_str() {
            edge_map.insert(id.to_string(), e);
        }
    }

    let node_rows: Vec<Value> = node_map.into_values().collect();
    let edge_rows: Vec<Value> = edge_map.into_values().collect();

    let tables = vec![
        TableRows {
            table: NODE_TABLE.to_string(),
            rows: node_rows,
        },
        TableRows {
            table: EDGE_TABLE.to_string(),
            rows: edge_rows,
        },
    ];

    let hint = GraphHint {
        graph_name: GRAPH_NAME.to_string(),
        node_table: NODE_TABLE.to_string(),
        edge_table: EDGE_TABLE.to_string(),
    };

    (tables, hint)
}

// ── import resolution ─────────────────────────────────────────────────────────

/// Resolve raw imports for `lang` to repo-relative paths where possible.
///
/// This is heuristic and best-effort.  Unresolvable imports (external crates,
/// stdlib, etc.) get `resolved_path = None` and become external `Module` nodes.
pub fn resolve_imports(
    lang: &str,
    file_path: &str,
    raw_imports: &[crate::github::parse::RawImport],
    all_paths: &std::collections::HashSet<String>,
) -> Vec<ImportTarget> {
    raw_imports
        .iter()
        .map(|ri| {
            let resolved_path = try_resolve(lang, file_path, &ri.raw, all_paths);
            ImportTarget {
                raw: ri.raw.clone(),
                line: ri.line,
                resolved_path,
            }
        })
        .collect()
}

/// Attempt to resolve a single raw import string to a repo path.
fn try_resolve(
    lang: &str,
    file_path: &str,
    raw: &str,
    all_paths: &std::collections::HashSet<String>,
) -> Option<String> {
    match lang {
        "rust" => resolve_rust(raw, all_paths),
        "python" => resolve_python(file_path, raw, all_paths),
        "typescript" | "javascript" => resolve_ts_js(file_path, raw, all_paths),
        "go" => resolve_go(raw, all_paths),
        _ => None,
    }
}

/// Rust: `crate::foo::bar` → `src/foo/bar.rs` or `src/foo/bar/mod.rs`.
/// `use std::` and other external crates → unresolved.
fn resolve_rust(raw: &str, all_paths: &std::collections::HashSet<String>) -> Option<String> {
    if !raw.starts_with("crate::") && !raw.starts_with("super::") {
        return None; // external crate
    }
    let inner = raw
        .strip_prefix("crate::")
        .or_else(|| raw.strip_prefix("super::"))
        .unwrap_or(raw);
    let rel = inner.replace("::", "/");
    // Try `src/{rel}.rs` and `src/{rel}/mod.rs`
    let candidate1 = format!("src/{rel}.rs");
    let candidate2 = format!("src/{rel}/mod.rs");
    if all_paths.contains(&candidate1) {
        return Some(candidate1);
    }
    if all_paths.contains(&candidate2) {
        return Some(candidate2);
    }
    None
}

/// Python: `from .rel import X` → sibling file; `from pkg import X` →
/// `pkg/__init__.py` or `pkg.py`.
fn resolve_python(
    file_path: &str,
    raw: &str,
    all_paths: &std::collections::HashSet<String>,
) -> Option<String> {
    let dir = std::path::Path::new(file_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    if raw.starts_with('.') {
        // Relative import — strip leading dots
        let stripped = raw.trim_start_matches('.');
        let base = if dir.is_empty() {
            stripped.replace('.', "/")
        } else {
            format!("{dir}/{}", stripped.replace('.', "/"))
        };
        let c1 = format!("{base}.py");
        let c2 = format!("{base}/__init__.py");
        if all_paths.contains(&c1) {
            return Some(c1);
        }
        if all_paths.contains(&c2) {
            return Some(c2);
        }
        return None;
    }

    // Absolute import
    let base = raw.replace('.', "/");
    let c1 = format!("{base}.py");
    let c2 = format!("{base}/__init__.py");
    if all_paths.contains(&c1) {
        return Some(c1);
    }
    if all_paths.contains(&c2) {
        return Some(c2);
    }
    None
}

/// TypeScript/JavaScript: `./foo` → sibling with any extension, relative to
/// the importing file's directory.
fn resolve_ts_js(
    file_path: &str,
    raw: &str,
    all_paths: &std::collections::HashSet<String>,
) -> Option<String> {
    if !raw.starts_with('.') {
        return None; // external npm package
    }
    let dir = std::path::Path::new(file_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let joined = if dir.is_empty() {
        raw.to_string()
    } else {
        format!("{dir}/{raw}")
    };
    // Normalize `./` and `../`
    let normalized = normalize_path(&joined);

    // Probe with various extensions
    for ext in &["ts", "tsx", "js", "jsx", "mjs"] {
        let c = if normalized.contains('.') && !normalized.ends_with('/') {
            // Already has extension
            normalized.clone()
        } else {
            format!("{normalized}.{ext}")
        };
        if all_paths.contains(&c) {
            return Some(c);
        }
    }
    // Try index file
    for ext in &["ts", "tsx", "js", "jsx"] {
        let c = format!("{normalized}/index.{ext}");
        if all_paths.contains(&c) {
            return Some(c);
        }
    }
    None
}

/// Go: `"github.com/owner/repo/pkg"` → check if any file at `pkg/*.go` exists
/// in the repo (module-relative heuristic).
fn resolve_go(raw: &str, all_paths: &std::collections::HashSet<String>) -> Option<String> {
    // Standard library (no dot in first segment) → unresolved
    if !raw.contains('/') {
        return None;
    }
    // For repo-internal imports: a Go module path typically starts with the
    // module name (e.g. `github.com/org/repo`).  We can't know the module name
    // from the tree alone, so we heuristically strip leading segments until we
    // find a directory that exists in the tree.
    let parts: Vec<&str> = raw.split('/').collect();
    for i in 1..parts.len() {
        let suffix = parts[i..].join("/");
        // Check if any .go file exists under this suffix directory
        let candidate_dir = suffix.clone();
        // Use a directory-boundary match (`candidate_dir/`) to avoid matching
        // paths that only share a prefix but are in a different directory
        // (e.g. `foo/` must not match `foobar/baz.go`).
        let dir_prefix = format!("{candidate_dir}/");
        let found = all_paths.iter().any(|p| {
            p.starts_with(&dir_prefix) && p.ends_with(".go")
        });
        if found {
            // Return the first .go file in this dir (deterministic via sorted set)
            return all_paths
                .iter()
                .find(|p| p.starts_with(&dir_prefix) && p.ends_with(".go"))
                .cloned();
        }
    }
    None
}

/// Naive path normalizer: remove `.` components and resolve `..`.
fn normalize_path(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_repo(owner: &str, name: &str) -> RawRecord {
        RawRecord::Repo(json!({
            "owner": { "login": owner },
            "name": name,
            "full_name": format!("{owner}/{name}"),
            "description": "test repo",
            "html_url": format!("https://github.com/{owner}/{name}"),
            "default_branch": "main",
            "private": false,
            "stargazers_count": 42,
        }))
    }

    fn make_user(login: &str) -> RawRecord {
        RawRecord::User(json!({
            "login": login,
            "html_url": format!("https://github.com/{login}"),
            "type": "User",
        }))
    }

    fn make_branch(owner: &str, repo: &str, branch: &str) -> RawRecord {
        RawRecord::Branch {
            owner: owner.to_string(),
            name: repo.to_string(),
            branch: json!({
                "name": branch,
                "commit": { "sha": "abc123" },
                "protected": false,
            }),
        }
    }

    fn make_pull(owner: &str, repo: &str, number: i64, title: &str, body: &str) -> RawRecord {
        RawRecord::Pull {
            owner: owner.to_string(),
            repo: repo.to_string(),
            pull: json!({
                "number": number,
                "title": title,
                "state": "open",
                "body": body,
                "user": { "login": "alice" },
                "head": { "ref": "feature-branch" },
                "base": { "ref": "main" },
                "html_url": format!("https://github.com/{owner}/{repo}/pull/{number}"),
            }),
        }
    }

    fn make_issue(owner: &str, repo: &str, number: i64, title: &str) -> RawRecord {
        RawRecord::Issue {
            owner: owner.to_string(),
            repo: repo.to_string(),
            issue: json!({
                "number": number,
                "title": title,
                "state": "open",
                "user": { "login": "bob" },
                "html_url": format!("https://github.com/{owner}/{repo}/issues/{number}"),
                "labels": [{ "name": "bug" }],
                "body": null,
            }),
        }
    }

    #[test]
    fn to_graph_basic_shape() {
        let records = vec![
            make_repo("acme", "myrepo"),
            make_user("alice"),
            make_pull("acme", "myrepo", 1, "My PR", ""),
            make_pull("acme", "myrepo", 2, "Second PR", ""),
            make_issue("acme", "myrepo", 10, "Bug report"),
        ];
        let (tables, hint) = to_graph(&records);
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[0].table, NODE_TABLE);
        assert_eq!(tables[1].table, EDGE_TABLE);
        assert_eq!(hint.graph_name, GRAPH_NAME);
        assert_eq!(hint.node_table, NODE_TABLE);
        assert_eq!(hint.edge_table, EDGE_TABLE);
    }

    #[test]
    fn node_column_count_le_4() {
        let records = vec![make_repo("a", "b"), make_user("alice")];
        let (tables, _) = to_graph(&records);
        for row in &tables[0].rows {
            let obj = row.as_object().unwrap();
            assert!(obj.len() <= 4, "node has {} columns: {:?}", obj.len(), obj.keys().collect::<Vec<_>>());
        }
    }

    #[test]
    fn edge_column_count_le_5() {
        let records = vec![make_repo("a", "b")];
        let (tables, _) = to_graph(&records);
        for row in &tables[1].rows {
            let obj = row.as_object().unwrap();
            assert!(obj.len() <= 5, "edge has {} columns", obj.len());
        }
    }

    #[test]
    fn deterministic_ids() {
        let records = vec![make_repo("acme", "myrepo"), make_user("alice")];
        let (tables1, _) = to_graph(&records);
        let (tables2, _) = to_graph(&records);
        let ids1: Vec<_> = tables1[0].rows.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        let ids2: Vec<_> = tables2[0].rows.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids1, ids2);
    }

    #[test]
    fn labels_is_single_string() {
        let records = vec![make_repo("a", "b")];
        let (tables, _) = to_graph(&records);
        for row in &tables[0].rows {
            let labels = &row["labels"];
            assert!(labels.is_string(), "labels should be a string, got {:?}", labels);
        }
    }

    #[test]
    fn closes_issue_edge_emitted() {
        let records = vec![
            make_repo("acme", "myrepo"),
            make_issue("acme", "myrepo", 5, "Some bug"),
            make_pull("acme", "myrepo", 1, "Fix bug", "Closes #5"),
        ];
        let (tables, _) = to_graph(&records);
        let edges = &tables[1].rows;
        let closes: Vec<_> = edges
            .iter()
            .filter(|e| e["type"] == "CLOSES_ISSUE")
            .collect();
        assert_eq!(closes.len(), 1);
        let src = closes[0]["src"].as_str().unwrap();
        let dst = closes[0]["dst"].as_str().unwrap();
        assert_eq!(src, pr_id("acme", "myrepo", 1));
        assert_eq!(dst, issue_id("acme", "myrepo", 5));
    }

    #[test]
    fn owns_edge_emitted() {
        let records = vec![make_repo("acme", "myrepo")];
        let (tables, _) = to_graph(&records);
        let edges = &tables[1].rows;
        let owns: Vec<_> = edges.iter().filter(|e| e["type"] == "OWNS").collect();
        assert_eq!(owns.len(), 1);
        assert_eq!(owns[0]["src"], user_id("acme"));
        assert_eq!(owns[0]["dst"], repo_id("acme", "myrepo"));
    }

    #[test]
    fn authored_edge_emitted_for_issue() {
        let records = vec![make_issue("acme", "myrepo", 1, "Bug")];
        let (tables, _) = to_graph(&records);
        let edges = &tables[1].rows;
        let authored: Vec<_> = edges.iter().filter(|e| e["type"] == "AUTHORED").collect();
        assert!(!authored.is_empty());
        assert_eq!(authored[0]["src"], user_id("bob"));
    }

    #[test]
    fn pr_stub_issues_skipped() {
        // An issue-like record that has a "pull_request" key should be ignored.
        let records = vec![RawRecord::Issue {
            owner: "a".into(),
            repo: "b".into(),
            issue: json!({
                "number": 99,
                "title": "PR stub",
                "pull_request": {},
                "user": { "login": "x" },
                "labels": [],
            }),
        }];
        let (tables, _) = to_graph(&records);
        let node_ids: Vec<_> = tables[0].rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert!(!node_ids.contains(&issue_id("a", "b", 99).as_str()));
    }

    #[test]
    fn branch_records_emitted() {
        let records = vec![
            make_repo("acme", "myrepo"),
            make_branch("acme", "myrepo", "main"),
        ];
        let (tables, _) = to_graph(&records);
        let node_ids: Vec<_> = tables[0].rows.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        assert!(node_ids.contains(&branch_id("acme", "myrepo", "main")));
        let edge_types: Vec<_> = tables[1].rows.iter().map(|e| e["type"].as_str().unwrap().to_string()).collect();
        assert!(edge_types.contains(&"HAS_BRANCH".to_string()));
    }

    #[test]
    fn props_is_valid_json_string() {
        let records = vec![make_repo("a", "b")];
        let (tables, _) = to_graph(&records);
        for row in &tables[0].rows {
            let props_str = row["props"].as_str().unwrap();
            let parsed: serde_json::Result<Value> = serde_json::from_str(props_str);
            assert!(parsed.is_ok(), "props is not valid JSON: {:?}", props_str);
        }
    }

    // ── Code graph tests (B2) ─────────────────────────────────────────────────

    fn make_file_tree() -> Vec<FileImports> {
        use crate::github::parse::{CodeSymbol, SymbolKind};
        vec![
            FileImports {
                path: "src/main.rs".into(),
                size: 100,
                language: "rust".into(),
                imports: vec![
                    ImportTarget {
                        raw: "crate::lib".into(),
                        line: 1,
                        resolved_path: Some("src/lib.rs".into()),
                    },
                    ImportTarget {
                        raw: "std::io".into(),
                        line: 2,
                        resolved_path: None,
                    },
                ],
                symbols: vec![
                    CodeSymbol { kind: SymbolKind::Function, name: "main".into(), line: 4 },
                    CodeSymbol { kind: SymbolKind::Class, name: "App".into(), line: 8 },
                ],
            },
            FileImports {
                path: "src/lib.rs".into(),
                size: 200,
                language: "rust".into(),
                imports: vec![],
                symbols: vec![],
            },
            FileImports {
                path: "src/helpers/mod.rs".into(),
                size: 50,
                language: "rust".into(),
                imports: vec![],
                symbols: vec![],
            },
        ]
    }

    #[test]
    fn code_graph_emits_symbols_and_defines() {
        let files = make_file_tree();
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let nodes = &tables[0].rows;
        let edges = &tables[1].rows;
        let fn_id = symbol_id("acme", "myrepo", "src/main.rs", "main", 4, "func");
        let cls_id = symbol_id("acme", "myrepo", "src/main.rs", "App", 8, "class");
        assert!(
            nodes.iter().any(|n| n["id"].as_str() == Some(fn_id.as_str()) && n["labels"].as_str() == Some("CodeFunction")),
            "missing CodeFunction node"
        );
        assert!(
            nodes.iter().any(|n| n["id"].as_str() == Some(cls_id.as_str()) && n["labels"].as_str() == Some("CodeClass")),
            "missing CodeClass node"
        );
        let file = file_id("acme", "myrepo", "src/main.rs");
        assert!(
            edges.iter().any(|e| e["type"].as_str() == Some("DEFINES")
                && e["src"].as_str() == Some(file.as_str())
                && e["dst"].as_str() == Some(fn_id.as_str())),
            "missing DEFINES edge file→function"
        );
        assert!(
            edges.iter().any(|e| e["type"].as_str() == Some("DEFINES") && e["dst"].as_str() == Some(cls_id.as_str())),
            "missing DEFINES edge file→class"
        );
    }

    #[test]
    fn code_graph_emits_directory_nodes() {
        let files = make_file_tree();
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let node_ids: Vec<_> = tables[0].rows.iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect();
        // Expect Directory nodes for "src" and "src/helpers"
        assert!(
            node_ids.contains(&dir_id("acme", "myrepo", "src")),
            "expected src dir, got: {:?}", node_ids
        );
        assert!(
            node_ids.contains(&dir_id("acme", "myrepo", "src/helpers")),
            "expected src/helpers dir, got: {:?}", node_ids
        );
    }

    #[test]
    fn code_graph_emits_codefile_nodes() {
        let files = make_file_tree();
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let node_ids: Vec<_> = tables[0].rows.iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect();
        assert!(
            node_ids.contains(&file_id("acme", "myrepo", "src/main.rs")),
            "expected src/main.rs file, got: {:?}", node_ids
        );
        assert!(
            node_ids.contains(&file_id("acme", "myrepo", "src/lib.rs")),
            "expected src/lib.rs file, got: {:?}", node_ids
        );
    }

    #[test]
    fn code_graph_codefile_labels_are_string() {
        let files = make_file_tree();
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        for row in &tables[0].rows {
            let labels = &row["labels"];
            assert!(labels.is_string(), "labels must be string, got: {:?}", labels);
        }
    }

    #[test]
    fn code_graph_node_columns_le_4() {
        let files = make_file_tree();
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        for row in &tables[0].rows {
            let obj = row.as_object().unwrap();
            assert!(
                obj.len() <= 4,
                "node has {} columns: {:?}",
                obj.len(),
                obj.keys().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn code_graph_edge_columns_le_5() {
        let files = make_file_tree();
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        for row in &tables[1].rows {
            let obj = row.as_object().unwrap();
            assert!(obj.len() <= 5, "edge has {} columns: {:?}", obj.len(), obj.keys().collect::<Vec<_>>());
        }
    }

    #[test]
    fn code_graph_contains_edges_repo_to_dir() {
        let files = make_file_tree();
        let rid = repo_id("acme", "myrepo");
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let edges = &tables[1].rows;
        // repo → src dir
        let src_did = dir_id("acme", "myrepo", "src");
        let repo_to_src = edges.iter().any(|e| {
            e["type"] == "CONTAINS" && e["src"] == rid && e["dst"] == src_did
        });
        assert!(repo_to_src, "expected CONTAINS repo→src, edges: {:?}",
            edges.iter().map(|e| (e["type"].as_str(), e["src"].as_str(), e["dst"].as_str())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn code_graph_contains_edges_dir_to_file() {
        let files = make_file_tree();
        let src_did = dir_id("acme", "myrepo", "src");
        let main_fid = file_id("acme", "myrepo", "src/main.rs");
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let edges = &tables[1].rows;
        let dir_to_file = edges.iter().any(|e| {
            e["type"] == "CONTAINS" && e["src"] == src_did && e["dst"] == main_fid
        });
        assert!(dir_to_file, "expected CONTAINS src→main.rs");
    }

    #[test]
    fn code_graph_imports_resolved_edge() {
        let files = make_file_tree();
        let main_fid = file_id("acme", "myrepo", "src/main.rs");
        let lib_fid = file_id("acme", "myrepo", "src/lib.rs");
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let edges = &tables[1].rows;
        let import_edge = edges.iter().find(|e| {
            e["type"] == "IMPORTS" && e["src"] == main_fid && e["dst"] == lib_fid
        });
        assert!(import_edge.is_some(), "expected IMPORTS main.rs→lib.rs");
        let props: Value = serde_json::from_str(
            import_edge.unwrap()["props"].as_str().unwrap()
        ).unwrap();
        assert_eq!(props["resolved"], true);
        assert_eq!(props["raw"], "crate::lib");
        assert_eq!(props["line"], 1);
    }

    #[test]
    fn code_graph_imports_unresolved_emits_module_node() {
        let files = make_file_tree();
        let main_fid = file_id("acme", "myrepo", "src/main.rs");
        let mod_id = module_id("std::io");
        let (tables, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);

        let edges = &tables[1].rows;
        let import_edge = edges.iter().find(|e| {
            e["type"] == "IMPORTS" && e["src"] == main_fid && e["dst"] == mod_id
        });
        assert!(import_edge.is_some(), "expected IMPORTS main.rs→module:std::io");
        let props: Value = serde_json::from_str(
            import_edge.unwrap()["props"].as_str().unwrap()
        ).unwrap();
        assert_eq!(props["resolved"], false);

        // Module node should exist
        let nodes = &tables[0].rows;
        let mod_node = nodes.iter().find(|n| n["id"] == mod_id);
        assert!(mod_node.is_some(), "expected Module node for std::io");
        assert_eq!(mod_node.unwrap()["labels"], "Module");
    }

    #[test]
    fn code_graph_deterministic() {
        let files = make_file_tree();
        let (t1, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let (t2, _) = code_to_graph("acme", "myrepo", &files, vec![], vec![]);
        let ids1: Vec<_> = t1[0].rows.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        let ids2: Vec<_> = t2[0].rows.iter().map(|r| r["id"].as_str().unwrap().to_string()).collect();
        assert_eq!(ids1, ids2);
    }

    // ── Import resolution tests ───────────────────────────────────────────────

    #[test]
    fn resolve_ts_js_relative_import() {
        let mut paths = std::collections::HashSet::new();
        paths.insert("src/utils.ts".to_string());
        paths.insert("src/index.ts".to_string());

        // ./utils from src/index.ts → src/utils.ts
        let imports = vec![crate::github::parse::RawImport { raw: "./utils".into(), line: 1 }];
        let resolved = resolve_imports("typescript", "src/index.ts", &imports, &paths);
        assert_eq!(resolved[0].resolved_path.as_deref(), Some("src/utils.ts"));
    }

    #[test]
    fn resolve_ts_js_external_npm_unresolved() {
        let paths = std::collections::HashSet::new();
        let imports = vec![crate::github::parse::RawImport { raw: "react".into(), line: 1 }];
        let resolved = resolve_imports("typescript", "src/index.ts", &imports, &paths);
        assert_eq!(resolved[0].resolved_path, None);
    }

    #[test]
    fn resolve_rust_crate_import() {
        let mut paths = std::collections::HashSet::new();
        paths.insert("src/utils.rs".to_string());

        let imports = vec![crate::github::parse::RawImport { raw: "crate::utils".into(), line: 1 }];
        let resolved = resolve_imports("rust", "src/main.rs", &imports, &paths);
        assert_eq!(resolved[0].resolved_path.as_deref(), Some("src/utils.rs"));
    }

    #[test]
    fn resolve_rust_std_unresolved() {
        let paths = std::collections::HashSet::new();
        let imports = vec![crate::github::parse::RawImport { raw: "std::io".into(), line: 1 }];
        let resolved = resolve_imports("rust", "src/main.rs", &imports, &paths);
        assert_eq!(resolved[0].resolved_path, None);
    }

    #[test]
    fn normalize_path_resolves_dotdot() {
        assert_eq!(normalize_path("src/../lib/foo"), "lib/foo");
        assert_eq!(normalize_path("src/./bar"), "src/bar");
        assert_eq!(normalize_path("a/b/c/../../d"), "a/d");
    }

    // ── Go import resolver tests ──────────────────────────────────────────────

    #[test]
    fn resolve_go_finds_package() {
        let mut paths = std::collections::HashSet::new();
        paths.insert("pkg/util/helper.go".to_string());

        let imports = vec![crate::github::parse::RawImport {
            raw: "github.com/acme/repo/pkg/util".into(),
            line: 1,
        }];
        let resolved = resolve_imports("go", "cmd/main.go", &imports, &paths);
        assert_eq!(
            resolved[0].resolved_path.as_deref(),
            Some("pkg/util/helper.go")
        );
    }

    #[test]
    fn resolve_go_no_prefix_match_false_positive() {
        // "foo" dir exists but import is for "foobar" — must not match.
        let mut paths = std::collections::HashSet::new();
        paths.insert("foo/main.go".to_string());

        let imports = vec![crate::github::parse::RawImport {
            raw: "github.com/acme/repo/foobar".into(),
            line: 1,
        }];
        let resolved = resolve_imports("go", "cmd/main.go", &imports, &paths);
        // "foobar/" prefix must not match "foo/main.go"
        assert_eq!(
            resolved[0].resolved_path,
            None,
            "foobar import must not resolve via foo/ directory"
        );
    }

    #[test]
    fn resolve_go_stdlib_unresolved() {
        let paths = std::collections::HashSet::new();
        let imports = vec![crate::github::parse::RawImport {
            raw: "fmt".into(),
            line: 1,
        }];
        let resolved = resolve_imports("go", "main.go", &imports, &paths);
        assert_eq!(resolved[0].resolved_path, None);
    }

    #[test]
    fn refetch_classification() {
        assert!(is_refetch_node(&make_node("repo:o/r", "Repository", "r", Map::new())));
        assert!(is_refetch_node(&make_node("user:o", "User", "o", Map::new())));
        assert!(is_refetch_node(&make_node("branch:o/r@main", "Branch", "main", Map::new())));
        assert!(!is_refetch_node(&make_node("pr:o/r#1", "PullRequest", "t", Map::new())));
        assert!(!is_refetch_node(&make_node("file:o/r:a.py", "CodeFile", "a.py", Map::new())));
        assert!(is_refetch_edge(&make_edge("user:o", "OWNS", "repo:o/r", Map::new())));
        assert!(is_refetch_edge(&make_edge("repo:o/r", "HAS_BRANCH", "branch:o/r@main", Map::new())));
        assert!(!is_refetch_edge(&make_edge("repo:o/r", "HAS_PULL_REQUEST", "pr:o/r#1", Map::new())));
    }

    #[test]
    fn refetch_signature_is_stable_order_independent_and_change_sensitive() {
        let mut p1 = Map::new();
        p1.insert("description".into(), Value::String("scripts".into()));
        let repo = make_node("repo:o/r", "Repository", "r", p1);
        let user = make_node("user:o", "User", "o", Map::new());
        let owns = make_edge("user:o", "OWNS", "repo:o/r", Map::new());
        // Non-refetch rows must not affect the signature.
        let pr = make_node("pr:o/r#1", "PullRequest", "t", Map::new());

        let sig_a = refetch_signature(&[repo.clone(), user.clone(), pr.clone()], &[owns.clone()]);
        // Reordered nodes + the PR removed → same signature.
        let sig_b = refetch_signature(&[user.clone(), repo.clone()], &[owns.clone()]);
        assert_eq!(sig_a, sig_b, "signature must be order-independent and ignore non-refetch rows");

        // Change a stored prop on the repo → signature changes.
        let mut p2 = Map::new();
        p2.insert("description".into(), Value::String("renamed".into()));
        let repo2 = make_node("repo:o/r", "Repository", "r", p2);
        let sig_c = refetch_signature(&[repo2, user], &[owns]);
        assert_ne!(sig_a, sig_c, "a changed stored prop must change the signature");
    }
}
