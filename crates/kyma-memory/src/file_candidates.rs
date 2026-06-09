//! File-candidate graph (E5).
//!
//! Coding agents (and the `kyma scrape`/watch worker) contribute files they
//! read; this module extracts their structure with the tree-sitter parser
//! (`kyma-codeparse`) and writes **candidate** `File` / `Symbol` / `Module`
//! nodes + `DEFINES` / `IMPORTS` / `REFERENCES` edges into an isolated
//! `file_candidates` graph — reusing the memory machinery (embeddings, recall,
//! graph registration) via [`MemoryWriter::with_database`]. A dreaming pipeline
//! (E6) later validates + promotes accepted candidates into the live graph.
//!
//! Persisting the *structure* (not the bytes) is what saves an agent's context
//! window: contribute a file once, then recall its meaning + relationships
//! cheaply via `describe_file` / `file_neighbors` instead of re-reading it.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::rows::{edge_row, preview};
use crate::{MemoryWriter, Result};

/// Database/namespace the candidate graph lives in (isolated from the live
/// `default`/`github`/`memory` graphs until E6 promotes accepted candidates).
pub const FILE_CANDIDATES_DB: &str = "file_candidates";

/// A file contributed by an agent / scraper. `content` must already be
/// secret-redacted by the caller.
#[derive(Debug, Clone)]
pub struct ContributeFile {
    pub path: String,
    pub realm: String,
    pub repo: Option<String>,
    pub content: String,
    pub content_sha256: String,
    /// Why the agent read it — folded into the File node's synopsis + embedding.
    pub why_read: Option<String>,
}

/// Summary of what one contribution wrote.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileContribution {
    pub file_node_id: String,
    pub language: String,
    pub symbols: usize,
    pub imports: usize,
    pub nodes: usize,
    pub edges: usize,
}

// ── deterministic ids (candidate namespace) ────────────────────────────────

pub fn file_node_id(realm: &str, repo: Option<&str>, path: &str) -> String {
    format!("cand:file:{realm}:{}:{path}", repo.unwrap_or(""))
}
pub fn symbol_node_id(file_id: &str, name: &str, line: usize, kind: &str) -> String {
    format!("cand:sym:{file_id}#{name}#{line}#{kind}")
}
pub fn module_node_id(realm: &str, raw: &str) -> String {
    format!("cand:mod:{realm}:{raw}")
}

#[allow(clippy::too_many_arguments)]
fn node_row(
    id: &str,
    label: &str,
    realm: &str,
    title: &str,
    content: &str,
    tags: &str,
    embedding: &[f32],
    provenance: &Value,
    now: &str,
) -> Value {
    json!({
        "id": id,
        "labels": label,
        "realm": realm,
        "memory_type": "entity",
        "title": title,
        "content": content,
        "content_preview": preview(content),
        "tags": tags,
        "importance": 0.5,
        "status": "active",
        "source_session_id": Value::Null,
        "source_run_id": Value::Null,
        "embedding": embedding,
        "created_at": now,
        "updated_at": now,
        "valid_at": now,
        "invalid_at": Value::Null,
        "superseded_by": Value::Null,
        "provenance": provenance.to_string(),
        "topic_key": Value::Null,
    })
}

/// Parse + write the candidate subgraph for one contributed file.
pub async fn contribute(writer: &MemoryWriter, req: &ContributeFile) -> Result<FileContribution> {
    let realm = if req.realm.is_empty() { "default" } else { req.realm.as_str() };
    let fid = file_node_id(realm, req.repo.as_deref(), &req.path);
    let now = chrono::Utc::now().to_rfc3339();

    let lang = kyma_codeparse::Lang::from_path(&req.path);
    let (symbols, imports, calls) = match lang {
        Some(l) => (
            kyma_codeparse::extract_symbols(l, &req.content),
            kyma_codeparse::extract_imports(l, &req.content),
            kyma_codeparse::extract_calls(l, &req.content),
        ),
        None => (vec![], vec![], vec![]),
    };
    let language = lang.map(|l| l.name().to_string()).unwrap_or_else(|| "text".to_string());

    // ── File node ──
    let sym_names: Vec<&str> = symbols.iter().take(12).map(|s| s.name.as_str()).collect();
    let imp_raws: Vec<&str> = imports.iter().take(12).map(|i| i.raw.as_str()).collect();
    let why = req
        .why_read
        .as_deref()
        .map(|w| format!(" Read because: {w}."))
        .unwrap_or_default();
    let defines = if sym_names.is_empty() {
        String::new()
    } else {
        format!(" Defines: {}.", sym_names.join(", "))
    };
    let uses = if imp_raws.is_empty() {
        String::new()
    } else {
        format!(" Imports: {}.", imp_raws.join(", "))
    };
    let synopsis = format!(
        "File {} ({language}) — {} symbol(s), {} import(s).{why}{defines}{uses}",
        req.path,
        symbols.len(),
        imports.len()
    );
    let file_prov = json!({
        "source": "file_contribution",
        "stage": "candidate",
        "repo": req.repo,
        "path": req.path,
        "language": language,
        "sha256": req.content_sha256,
        "why_read": req.why_read,
        "symbol_count": symbols.len(),
        "import_count": imports.len(),
    });

    // Collect node specs: (id, label, title, content, tags, provenance). Embed
    // all texts in one batch.
    struct Spec {
        id: String,
        label: &'static str,
        title: String,
        content: String,
        tags: String,
        prov: Value,
    }
    let mut specs: Vec<Spec> = vec![Spec {
        id: fid.clone(),
        label: "File",
        title: req.path.clone(),
        content: synopsis,
        tags: format!("file,{language}"),
        prov: file_prov,
    }];

    // Symbols → Symbol nodes (+ name→id for intra-file call edges).
    let mut sym_id_by_name: HashMap<String, String> = HashMap::new();
    for s in &symbols {
        let kind = match s.kind {
            kyma_codeparse::SymbolKind::Function => "func",
            kyma_codeparse::SymbolKind::Class => "class",
        };
        let sid = symbol_node_id(&fid, &s.name, s.line, kind);
        sym_id_by_name.entry(s.name.clone()).or_insert_with(|| sid.clone());
        specs.push(Spec {
            id: sid,
            label: "Symbol",
            title: s.name.clone(),
            content: format!("{kind} {} in {}", s.name, req.path),
            tags: format!("symbol,{kind},{language}"),
            prov: json!({"stage": "candidate", "kind": kind, "line": s.line, "file": fid}),
        });
    }

    // Imports → Module nodes (deduped).
    let mut module_ids: HashSet<String> = HashSet::new();
    for i in &imports {
        let mid = module_node_id(realm, &i.raw);
        if module_ids.insert(mid.clone()) {
            specs.push(Spec {
                id: mid,
                label: "Module",
                title: i.raw.clone(),
                content: format!("Module/import target {}", i.raw),
                tags: "module".to_string(),
                prov: json!({"stage": "candidate", "raw": i.raw}),
            });
        }
    }

    // Embed every node's content in one round-trip.
    let texts: Vec<String> = specs.iter().map(|s| s.content.clone()).collect();
    let embeddings = writer.embed_batch(&texts).await?;

    let node_rows: Vec<Value> = specs
        .iter()
        .zip(embeddings.iter())
        .map(|(s, emb)| node_row(&s.id, s.label, realm, &s.title, &s.content, &s.tags, emb, &s.prov, &now))
        .collect();

    // ── Edges ──
    let mut edge_rows: Vec<Value> = Vec::new();
    for s in &symbols {
        let kind = match s.kind {
            kyma_codeparse::SymbolKind::Function => "func",
            kyma_codeparse::SymbolKind::Class => "class",
        };
        let sid = symbol_node_id(&fid, &s.name, s.line, kind);
        edge_rows.push(edge_row(&fid, &sid, "DEFINES", realm, None, None, &now));
    }
    for mid in &module_ids {
        edge_rows.push(edge_row(&fid, mid, "IMPORTS", realm, None, None, &now));
    }
    // Intra-file calls → REFERENCES (caller symbol → callee symbol), only when
    // both ends resolve to a symbol defined in this file.
    let mut seen_calls: HashSet<(String, String)> = HashSet::new();
    for c in &calls {
        let (Some(caller), Some(callee_id)) = (
            c.caller.as_ref().and_then(|n| sym_id_by_name.get(n)),
            sym_id_by_name.get(&c.callee),
        ) else {
            continue;
        };
        if caller == callee_id {
            continue;
        }
        if seen_calls.insert((caller.clone(), callee_id.clone())) {
            edge_rows.push(edge_row(caller, callee_id, "REFERENCES", realm, None, None, &now));
        }
    }

    let nodes = node_rows.len();
    let edges = edge_rows.len();
    writer.append_node_rows(node_rows).await?;
    writer.append_edge_rows(edge_rows).await?;

    Ok(FileContribution {
        file_node_id: fid,
        language,
        symbols: symbols.len(),
        imports: imports.len(),
        nodes,
        edges,
    })
}
