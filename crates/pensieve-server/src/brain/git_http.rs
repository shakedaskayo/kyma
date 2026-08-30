//! Git smart-HTTP service for brain repos: the grack/GitLab stateless-rpc
//! pattern over the `git` binary.
//!
//! ```text
//! GET  /git/:repo/info/refs?service=git-upload-pack|git-receive-pack
//! POST /git/:repo/git-upload-pack        (clone/fetch — Role::Read)
//! POST /git/:repo/git-receive-pack       (push — Role::Write + ingest)
//! ```
//!
//! A push is also the ingest trigger: after `receive-pack` updates the
//! branch, the old..new diff is planned by `kyma_brain::ingest` and applied
//! through the `MemoryWriter` while the per-brain lock is held, so a
//! subsequent export can never miss a landed-but-uningested push.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Extension;
use chrono::Utc;
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read as _;
use tokio::io::AsyncWriteExt as _;

use kyma_brain::gitbin::GitBin;
use kyma_brain::ingest::{plan_push_ingest, IngestOp};
use kyma_brain::registry::{BrainRecord, BrainRunRecord};
use kyma_brain::{pktline, BRAIN_BRANCH};

use crate::agent::tools::{execute_sql, SharedToolCtx};
use crate::auth::{Principal, Role};

use super::BrainState;

/// Max buffered request body (upload-pack negotiation / pushed packfiles).
const MAX_BODY_BYTES: usize = 256 * 1024 * 1024;

/// Router builder — caller layers [`crate::auth::require_git_auth_middleware`].
pub fn git_http_router(state: BrainState) -> axum::Router {
    axum::Router::new()
        .route("/git/:repo/info/refs", get(info_refs_handler))
        .route("/git/:repo/git-upload-pack", post(upload_pack_handler))
        .route("/git/:repo/git-receive-pack", post(receive_pack_handler))
        .with_state(state)
}

fn plain(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, msg.into()).into_response()
}

fn role_for(name: &str) -> Role {
    match name {
        "admin" => Role::Admin,
        "write" => Role::Write,
        _ => Role::Read,
    }
}

/// Resolve `:repo` (`<name>.git`) to a registered brain, enforcing the
/// per-brain visibility role.
async fn resolve_brain(
    state: &BrainState,
    repo: &str,
    principal: &Principal,
    needed: Role,
) -> Result<(BrainRecord, std::path::PathBuf, std::sync::Arc<GitBin>), Response> {
    let Some(git) = state.git.clone() else {
        return Err(plain(StatusCode::SERVICE_UNAVAILABLE, "git binary not found on server host"));
    };
    let Some(name) = repo.strip_suffix(".git") else {
        return Err(plain(StatusCode::NOT_FOUND, "repository must be addressed as <name>.git"));
    };
    if kyma_brain::validate_name(name).is_err() {
        return Err(plain(StatusCode::NOT_FOUND, "unknown repository"));
    }
    let rec = match state.registry.get(name).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(plain(StatusCode::NOT_FOUND, "unknown repository")),
        Err(e) => return Err(plain(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };
    let visibility = role_for(&rec.config.visibility_role);
    let required = if needed > visibility { needed } else { visibility };
    if principal.role < required {
        return Err(plain(StatusCode::FORBIDDEN, "token role insufficient for this repository"));
    }
    let dir = state.repo_dir(name);
    if !dir.exists() {
        return Err(plain(StatusCode::NOT_FOUND, "repository not initialized"));
    }
    Ok((rec, dir, git))
}

fn no_cache_headers(content_type: &str) -> [(header::HeaderName, String); 4] {
    [
        (header::CONTENT_TYPE, content_type.to_string()),
        (header::CACHE_CONTROL, "no-cache, max-age=0, must-revalidate".to_string()),
        (header::PRAGMA, "no-cache".to_string()),
        (header::EXPIRES, "Fri, 01 Jan 1980 00:00:00 GMT".to_string()),
    ]
}

fn git_protocol(headers: &HeaderMap) -> Option<String> {
    headers.get("git-protocol").and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// Run a stateless-rpc child to completion with `input` on stdin, returning
/// stdout. Push/pull payloads are buffered — brain repos are modest; B4
/// hardening streams instead.
async fn run_service(
    git: &GitBin,
    dir: &std::path::Path,
    service: &str,
    advertise: bool,
    proto: Option<&str>,
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>, Response> {
    let mut child = git
        .spawn_service(dir, service, advertise, proto)
        .map_err(|e| plain(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if let Some(bytes) = input {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| plain(StatusCode::INTERNAL_SERVER_ERROR, "stdin pipe missing"))?;
        let write = async move {
            stdin.write_all(&bytes).await?;
            stdin.shutdown().await
        };
        if let Err(e) = write.await {
            return Err(plain(StatusCode::INTERNAL_SERVER_ERROR, format!("git stdin: {e}")));
        }
    } else {
        drop(child.stdin.take());
    }
    let out = tokio::time::timeout(std::time::Duration::from_secs(900), child.wait_with_output())
        .await
        .map_err(|_| plain(StatusCode::GATEWAY_TIMEOUT, "git service timed out"))?
        .map_err(|e| plain(StatusCode::INTERNAL_SERVER_ERROR, format!("git service: {e}")))?;
    if !out.status.success() {
        return Err(plain(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("git {service} failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        ));
    }
    Ok(out.stdout)
}

async fn read_body(headers: &HeaderMap, body: Body) -> Result<Vec<u8>, Response> {
    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|e| plain(StatusCode::PAYLOAD_TOO_LARGE, format!("request body: {e}")))?;
    let gzipped = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("gzip"));
    if !gzipped {
        return Ok(bytes.to_vec());
    }
    let mut out = Vec::with_capacity(bytes.len() * 4);
    GzDecoder::new(bytes.as_ref())
        .read_to_end(&mut out)
        .map_err(|e| plain(StatusCode::BAD_REQUEST, format!("gzip body: {e}")))?;
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct ServiceParam {
    #[serde(default)]
    service: Option<String>,
}

async fn info_refs_handler(
    State(state): State<BrainState>,
    Path(repo): Path<String>,
    Query(params): Query<ServiceParam>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> Response {
    let service = params.service.unwrap_or_default();
    let needed = match service.as_str() {
        "git-upload-pack" => Role::Read,
        "git-receive-pack" => Role::Write,
        // Dumb-protocol clients omit ?service= — unsupported on purpose.
        _ => return plain(StatusCode::FORBIDDEN, "smart HTTP only: pass ?service=git-upload-pack"),
    };
    let (_rec, dir, git) = match resolve_brain(&state, &repo, &principal, needed).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let out = match run_service(&git, &dir, &service, true, git_protocol(&headers).as_deref(), None)
        .await
    {
        Ok(o) => o,
        Err(r) => return r,
    };
    let mut body = pktline::service_banner(&service);
    body.extend_from_slice(&out);
    (no_cache_headers(&format!("application/x-{service}-advertisement")), body).into_response()
}

async fn upload_pack_handler(
    State(state): State<BrainState>,
    Path(repo): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (_rec, dir, git) = match resolve_brain(&state, &repo, &principal, Role::Read).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let input = match read_body(&headers, body).await {
        Ok(b) => b,
        Err(r) => return r,
    };
    match run_service(&git, &dir, "git-upload-pack", false, git_protocol(&headers).as_deref(), Some(input))
        .await
    {
        Ok(out) => {
            (no_cache_headers("application/x-git-upload-pack-result"), out).into_response()
        }
        Err(r) => r,
    }
}

async fn receive_pack_handler(
    State(state): State<BrainState>,
    Path(repo): Path<String>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (rec, dir, git) = match resolve_brain(&state, &repo, &principal, Role::Write).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let input = match read_body(&headers, body).await {
        Ok(b) => b,
        Err(r) => return r,
    };

    // Serialize against exports and other pushes for the whole
    // receive → diff → ingest window.
    let lock = state.lock_for(&rec.config.name);
    let _guard = lock.lock().await;

    let refname = format!("refs/heads/{BRAIN_BRANCH}");
    let old_head = match git.rev_parse(&dir, &refname).await {
        Ok(h) => h,
        Err(e) => return plain(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };

    let out = match run_service(
        &git,
        &dir,
        "git-receive-pack",
        false,
        git_protocol(&headers).as_deref(),
        Some(input),
    )
    .await
    {
        Ok(o) => o,
        Err(r) => return r,
    };

    // Ingest whatever landed (receive-pack reports per-ref results to the
    // client in `out`; a rejected push simply produces no ref movement).
    let new_head = git.rev_parse(&dir, &refname).await.ok().flatten();
    if let Some(new) = new_head {
        if old_head.as_deref() != Some(new.as_str()) {
            let started = Utc::now().to_rfc3339();
            let report = ingest_push(&state, &rec, &git, &dir, old_head.as_deref(), &new).await;
            record_push_run(&state, &rec.config.name, started, report).await;
        }
    }

    (no_cache_headers("application/x-git-receive-pack-result"), out).into_response()
}

struct PushIngestReport {
    ingested: u64,
    warnings: Vec<String>,
    error: Option<String>,
}

async fn record_push_run(
    state: &BrainState,
    name: &str,
    started: String,
    report: PushIngestReport,
) {
    let Ok(Some(mut rec)) = state.registry.get(name).await else { return };
    if let Some(e) = &report.error {
        rec.runtime.last_error = Some(e.clone());
    }
    rec.runtime.record_run(BrainRunRecord {
        kind: "push_ingest".into(),
        started_at: started,
        finished_at: Utc::now().to_rfc3339(),
        commit: None,
        files_written: 0,
        files_deleted: 0,
        notes_ingested: report.ingested,
        noop: report.ingested == 0 && report.error.is_none(),
        error: report.error,
        warnings: report.warnings,
    });
    let _ = state.registry.update_runtime(name, &rec.runtime).await;
}

fn shared_ctx(state: &BrainState) -> SharedToolCtx {
    SharedToolCtx {
        realm_scope: Default::default(),
        consumer_sink: None,
        federation: None,
        catalog: state.agent.catalog.clone(),
        format: state.agent.format.clone(),
        pool: state.agent.pool.clone(),
        memory: state.agent.memory.clone(),
        hitl: None,
        memory_settings_path: state.agent.memory_settings_path.clone(),
    }
}

/// Latest full node row by bare memory uuid (all columns — re-appended
/// verbatim on archive, embedding included).
async fn latest_row_by_id(state: &BrainState, id: &str) -> Option<Value> {
    let node_id = kyma_memory::sql::sql_str(&format!("memory:{id}"));
    let q = format!(
        "WITH latest AS (SELECT *, row_number() OVER (PARTITION BY id ORDER BY updated_at DESC) AS __rn \
         FROM memory_nodes) SELECT * FROM latest WHERE __rn = 1 AND id = {node_id} LIMIT 1"
    );
    execute_sql(&shared_ctx(state), kyma_memory::DEFAULT_DATABASE, &q, 1)
        .await
        .get("rows")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .cloned()
}

async fn ingest_push(
    state: &BrainState,
    rec: &BrainRecord,
    git: &GitBin,
    dir: &std::path::Path,
    old_head: Option<&str>,
    new_head: &str,
) -> PushIngestReport {
    let mut report = PushIngestReport { ingested: 0, warnings: Vec::new(), error: None };

    // Diff old..new (empty-tree base for the first push into an empty repo).
    let changes = match old_head {
        Some(old) => git.diff_name_status(dir, old, new_head).await,
        None => git.ls_tree_paths(dir, new_head).await.map(|paths| {
            paths
                .into_iter()
                .map(|p| (kyma_brain::gitbin::ChangeKind::Added, p))
                .collect::<Vec<_>>()
        }),
    };
    let changes = match changes {
        Ok(c) => c,
        Err(e) => {
            report.error = Some(format!("diff: {e}"));
            return report;
        }
    };
    if changes.is_empty() {
        return report;
    }

    // Prior manifest (pre-push tip) maps exported paths to memory ids.
    let prior = match old_head {
        Some(old) => kyma_brain::export::read_prior_manifest(git, dir, Some(old)).await,
        None => Ok(kyma_brain::types::Manifest::default()),
    }
    .unwrap_or_default();
    let prior_ids: BTreeMap<String, String> = prior.memory_ids_by_path();

    // Blob reads are synchronous inside the pure planner, so pre-read every
    // changed path at the new tip.
    let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (kind, path) in &changes {
        if !matches!(kind, kyma_brain::gitbin::ChangeKind::Deleted) && path.ends_with(".md") {
            if let Ok(bytes) = git.cat_file(dir, new_head, path).await {
                blobs.insert(path.clone(), bytes);
            }
        }
    }
    let plan = plan_push_ingest(&rec.config, &changes, |p| blobs.get(p).cloned(), &prior_ids);
    report.warnings = plan.warnings;
    if plan.ops.is_empty() {
        return report;
    }

    let embed = match kyma_memory::shared_embedding().await {
        Ok(e) => e,
        Err(e) => {
            report.error = Some(format!("embedding backend: {e}"));
            return report;
        }
    };
    let writer = kyma_memory::MemoryWriter::new(
        state.agent.catalog.clone(),
        state.agent.format.clone(),
        embed,
    );

    let now = Utc::now().to_rfc3339();
    for op in plan.ops {
        let result = apply_op(state, &writer, rec, &now, op).await;
        match result {
            Ok(()) => report.ingested += 1,
            Err(w) => report.warnings.push(w),
        }
    }
    report
}

fn create_memory_from_note(
    rec: &BrainRecord,
    note: &kyma_brain::notes::ParsedNote,
    realm: String,
    topic_key: Option<String>,
    rel_path: &str,
) -> kyma_memory::CreateMemory {
    let mut m = kyma_memory::CreateMemory::new(note.body.clone());
    m.title = note.title.clone();
    m.memory_type = note
        .memory_type
        .as_deref()
        .map_or(kyma_memory::MemoryType::Fact, kyma_memory::MemoryType::parse);
    m.tags = note.tags.clone();
    m.realm = realm;
    m.importance = note.importance.map_or(0.6, |v| v as f32);
    m.topic_key = topic_key;
    m.provenance = Some(json!({
        "source": kyma_brain::BRAIN_PROVENANCE_SOURCE,
        "brain": rec.config.name,
        "path": rel_path,
    }));
    m
}

async fn apply_op(
    state: &BrainState,
    writer: &kyma_memory::MemoryWriter,
    rec: &BrainRecord,
    now: &str,
    op: IngestOp,
) -> Result<(), String> {
    match op {
        IngestOp::UpdateExisting { memory_id, rel_path, note } => {
            let uuid = uuid::Uuid::parse_str(&memory_id)
                .map_err(|_| format!("{rel_path}: invalid kyma_memory_id, skipped"))?;
            let existing = latest_row_by_id(state, &memory_id).await;
            let Some(row) = existing else {
                return Err(format!("{rel_path}: memory {memory_id} not found, skipped"));
            };
            // Preserve identity fields the note doesn't (or must not) carry.
            let field = |k: &str| row.get(k).and_then(Value::as_str).map(str::to_string);
            let realm = field("realm").unwrap_or_else(|| "default".to_string());
            let mut m = create_memory_from_note(
                rec,
                &note,
                realm,
                field("topic_key").filter(|s| !s.is_empty()),
                &rel_path,
            );
            if m.title.is_none() {
                m.title = field("title");
            }
            if note.memory_type.is_none() {
                if let Some(t) = field("memory_type") {
                    m.memory_type = kyma_memory::MemoryType::parse(&t);
                }
            }
            if note.importance.is_none() {
                if let Some(i) = row.get("importance").and_then(Value::as_f64) {
                    m.importance = i as f32;
                }
            }
            if note.tags.is_empty() {
                if let Some(tags) = field("tags") {
                    m.tags = tags
                        .split(',')
                        .map(str::trim)
                        .filter(|t| !t.is_empty())
                        .map(str::to_string)
                        .collect();
                }
            }
            writer.save_as(uuid, &m).await.map_err(|e| format!("{rel_path}: {e}"))?;
            Ok(())
        }
        IngestOp::CreateNew { topic_key, realm, rel_path, note } => {
            let m = create_memory_from_note(rec, &note, realm, Some(topic_key), &rel_path);
            writer.save(&m).await.map_err(|e| format!("{rel_path}: {e}"))?;
            Ok(())
        }
        IngestOp::ArchiveDeleted { memory_id, rel_path } => {
            let Some(mut row) = latest_row_by_id(state, &memory_id).await else {
                return Err(format!("{rel_path}: memory {memory_id} not found for archive"));
            };
            if row.get("status").and_then(Value::as_str) == Some("archived") {
                return Ok(());
            }
            let mut prov = row
                .get("provenance")
                .and_then(Value::as_str)
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or_else(|| json!({}));
            if let Some(obj) = prov.as_object_mut() {
                obj.insert("brain_archived_reason".into(), json!("file_deleted_on_push"));
                obj.insert("brain_archived_at".into(), json!(now));
                obj.insert("brain".into(), json!(rec.config.name));
            }
            if let Some(obj) = row.as_object_mut() {
                obj.insert("status".into(), json!("archived"));
                obj.insert("invalid_at".into(), json!(now));
                obj.insert("updated_at".into(), json!(now));
                obj.insert("provenance".into(), json!(prov.to_string()));
            }
            writer
                .append_node_rows(vec![row])
                .await
                .map_err(|e| format!("{rel_path}: archive: {e}"))?;
            Ok(())
        }
    }
}
