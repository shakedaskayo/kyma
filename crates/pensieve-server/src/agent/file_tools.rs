//! Agent-facing file-contribution + recall tools (E5).
//!
//! The context-window economy loop: an agent `contribute_file`s a file it read
//! once (parsed into a candidate File/Symbol/Module subgraph), then recalls its
//! meaning + relationships cheaply via `describe_file` / `file_neighbors` /
//! `recall_file` instead of re-reading the bytes.

use std::sync::Arc;

use adk_rust::tool::FunctionTool;
use adk_rust::{Tool, ToolContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use pensieve_core::catalog::Catalog;
use pensieve_core::segment_format::SegmentFormat;
use pensieve_memory::file_candidates::{self, ContributeFile, FileContribution, FILE_CANDIDATES_DB};
use pensieve_memory::MemoryWriter;

use super::tools::{execute_sql, SharedToolCtx};

/// Shared contribution path used by both the `contribute_file` MCP tool and the
/// `POST /v1/agent/files/contribute` HTTP endpoint (which `pensieve scrape`/`watch`
/// call). Redacts secrets, hashes, parses, and writes the candidate subgraph.
pub async fn contribute_file_impl(
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    path: String,
    realm: String,
    repo: Option<String>,
    content: String,
    why_read: Option<String>,
) -> std::result::Result<FileContribution, String> {
    let (redacted, _findings) = pensieve_redact::global().redact_text(&content);
    let sha = sha256_hex(redacted.as_bytes());
    let embed = pensieve_memory::shared_embedding()
        .await
        .map_err(|e| format!("embedding backend: {e}"))?;
    let writer = MemoryWriter::new(catalog, format, embed).with_database(FILE_CANDIDATES_DB);
    let req = ContributeFile {
        path,
        realm,
        repo,
        content: redacted,
        content_sha256: sha,
        why_read,
    };
    file_candidates::contribute(&writer, &req)
        .await
        .map_err(|e| format!("contribute: {e}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::new().chain_update(bytes).finalize();
    let mut s = String::with_capacity(64);
    for b in d {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

use super::memory::sql_lit;

// ── contribute_file ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct ContributeArgs {
    /// Repo-relative path, e.g. "crates/foo/src/bar.rs". Drives language + ids.
    path: String,
    /// Project realm (defaults to "default").
    #[serde(default)]
    realm: String,
    /// "owner/name" if the file belongs to a known repo (enables E6 cross-graph
    /// resolution to the live code graph). Optional.
    repo: Option<String>,
    /// Full file content. Secrets are redacted before anything is persisted.
    content: String,
    /// Why you read it — folded into the File node's synopsis + embedding.
    why_read: Option<String>,
}

const CONTRIBUTE_DESC: &str = "Contribute a file you read so pensieve persists its \
structure (symbols, imports, call edges) as candidate graph nodes. Do this once \
per file; afterwards use describe_file / file_neighbors / recall_file to recall \
its meaning + relationships cheaply instead of re-reading it. Secrets in the \
content are redacted before storage.";

pub fn tool_contribute_file(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "contribute_file",
            CONTRIBUTE_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let a: ContributeArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    // Realm scope: gate the file-graph write. Normalize the
                    // empty serde-default to the same "default" the impl uses.
                    let eff_realm = if a.realm.is_empty() { "default" } else { &a.realm };
                    if let Some(err) = shared.check_realm_write(eff_realm) {
                        return Ok(err);
                    }
                    match contribute_file_impl(
                        shared.catalog.clone(),
                        shared.format.clone(),
                        a.path,
                        a.realm,
                        a.repo,
                        a.content,
                        a.why_read,
                    )
                    .await
                    {
                        Ok(s) => Ok(serde_json::to_value(s).unwrap_or_else(|_| json!({}))),
                        Err(e) => Ok(json!({"error": e})),
                    }
                }
            },
        )
        .with_parameters_schema::<ContributeArgs>()
        .with_read_only(false)
        .with_concurrency_safe(true),
    )
}

// ── describe_file ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct FileRefArgs {
    /// Repo-relative path used to contribute the file.
    path: String,
    #[serde(default)]
    realm: String,
    repo: Option<String>,
}

const DESCRIBE_DESC: &str = "Recall a contributed file's meaning WITHOUT its \
bytes: a synopsis listing the symbols it defines and modules it imports, plus \
provenance (language, sha, counts). Cheap — use instead of re-reading the file.";

pub fn tool_describe_file(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "describe_file",
            DESCRIBE_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let a: FileRefArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let realm = if a.realm.is_empty() { "default" } else { &a.realm };
                    // Realm scope: file-candidate reads are single-realm; deny
                    // an out-of-scope realm rather than reading it.
                    if !shared.realm_scope.allows(realm) {
                        return Ok(json!({
                            "error": format!("token not scoped to realm `{realm}`"),
                            "code": "realm_forbidden",
                        }));
                    }
                    let fid = file_candidates::file_node_id(realm, a.repo.as_deref(), &a.path);
                    let sql = format!(
                        "WITH latest AS (SELECT id, title, content, provenance, \
                         row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn \
                         FROM memory_nodes WHERE id = '{}') \
                         SELECT title, content, provenance FROM latest WHERE rn = 1",
                        sql_lit(&fid)
                    );
                    let res = execute_sql(&shared, FILE_CANDIDATES_DB, &sql, 1).await;
                    let rows = res.get("rows").and_then(Value::as_array).cloned().unwrap_or_default();
                    if rows.is_empty() {
                        return Ok(json!({"found": false, "file_node_id": fid}));
                    }
                    let r = &rows[0];
                    Ok(json!({
                        "found": true,
                        "file_node_id": fid,
                        "synopsis": r.get("content"),
                        "provenance": r.get("provenance"),
                    }))
                }
            },
        )
        .with_parameters_schema::<FileRefArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ── file_neighbors ──────────────────────────────────────────────────────────

const NEIGHBORS_DESC: &str = "List a contributed file's relationships from the \
candidate graph: the symbols it DEFINES, the modules it IMPORTS, and intra-file \
REFERENCES (call edges) — by id + name, no bytes.";

pub fn tool_file_neighbors(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "file_neighbors",
            NEIGHBORS_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let a: FileRefArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let realm = if a.realm.is_empty() { "default" } else { &a.realm };
                    // Realm scope: file-candidate reads are single-realm; deny
                    // an out-of-scope realm rather than reading it.
                    if !shared.realm_scope.allows(realm) {
                        return Ok(json!({
                            "error": format!("token not scoped to realm `{realm}`"),
                            "code": "realm_forbidden",
                        }));
                    }
                    let fid = file_candidates::file_node_id(realm, a.repo.as_deref(), &a.path);
                    // Edges out of the file, joined to the latest version of each
                    // neighbour for its human-readable title.
                    let sql = format!(
                        "WITH n AS (SELECT id, title, \
                           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn \
                           FROM memory_nodes) \
                         SELECT e.type AS relationship, e.dst AS target_id, n.title AS target_name \
                         FROM memory_edges e LEFT JOIN n ON n.id = e.dst AND n.rn = 1 \
                         WHERE e.src = '{}' \
                         ORDER BY relationship",
                        sql_lit(&fid)
                    );
                    let res = execute_sql(&shared, FILE_CANDIDATES_DB, &sql, 500).await;
                    let neighbours = res.get("rows").cloned().unwrap_or_else(|| json!([]));
                    Ok(json!({ "file_node_id": fid, "neighbours": neighbours }))
                }
            },
        )
        .with_parameters_schema::<FileRefArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ── recall_file ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct RecallArgs {
    /// Natural-language description of the file you're looking for.
    query: String,
    /// Max results (default 8).
    #[serde(default = "default_recall_limit")]
    limit: usize,
}
fn default_recall_limit() -> usize {
    8
}

const RECALL_DESC: &str = "Find contributed files by meaning (vector search over \
their synopses), e.g. \"where is the import-edge builder?\". Returns ranked File \
nodes (path + synopsis preview) — then describe_file / file_neighbors to drill in.";

pub fn tool_recall_file(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "recall_file",
            RECALL_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let a: RecallArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let embed = match pensieve_memory::shared_embedding().await {
                        Ok(e) => e,
                        Err(e) => return Ok(json!({"error": format!("embedding backend: {e}")})),
                    };
                    let vec = match embed.embed(&[a.query.clone()]).await {
                        Ok(mut v) if !v.is_empty() => v.remove(0),
                        Ok(_) => return Ok(json!({"error": "empty embedding"})),
                        Err(e) => return Ok(json!({"error": format!("embed: {e}")})),
                    };
                    let arr = vec
                        .iter()
                        .map(|f| format!("{f:.6}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    let sql = format!(
                        "WITH latest AS (SELECT id, title, content_preview, embedding, \
                           row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS rn \
                           FROM memory_nodes WHERE labels = 'File') \
                         SELECT id, title, content_preview, \
                                cosine_distance(embedding, make_array({arr})) AS distance \
                         FROM latest WHERE rn = 1 ORDER BY distance ASC LIMIT {}",
                        a.limit.min(50)
                    );
                    let res = execute_sql(&shared, FILE_CANDIDATES_DB, &sql, a.limit.min(50)).await;
                    let files = res.get("rows").cloned().unwrap_or_else(|| json!([]));
                    Ok(json!({ "files": files }))
                }
            },
        )
        .with_parameters_schema::<RecallArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}
