//! `/v1/brain` management API: registry CRUD, export trigger, run history,
//! clone guidance. Mounted behind `Role::Read` middleware like the rest of
//! the agent surface; mutating routes gate stricter roles in-handler (the
//! `import_memory_handler` pattern).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use kyma_brain::registry::{
    BrainConfig, BrainFilters, BrainRunRecord, GardenerConfig, RealmSelector,
};

use crate::auth::{Principal, Role};

use super::{fetch, BrainState};

/// Router builder — caller layers auth middleware (house style).
pub fn brain_router(state: BrainState) -> axum::Router {
    axum::Router::new()
        .route("/v1/brain", get(list_handler).post(create_handler))
        .route(
            "/v1/brain/:name",
            get(get_handler).delete(delete_handler),
        )
        .route("/v1/brain/:name", put(update_handler))
        .route("/v1/brain/:name/export", post(export_handler))
        .route("/v1/brain/:name/garden", post(garden_handler))
        .route("/v1/brain/:name/runs", get(runs_handler))
        .route("/v1/brain/:name/clone-info", get(clone_info_handler))
        .route("/v1/brain/:name/tree", get(tree_handler))
        .route("/v1/brain/:name/file", get(file_handler))
        .with_state(state)
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn require_role(principal: &Principal, needed: Role, what: &str) -> Option<Response> {
    if principal.role < needed {
        return Some(err(StatusCode::FORBIDDEN, format!("{what} requires {needed:?} role")));
    }
    None
}

fn brain_json(state: &BrainState, rec: &kyma_brain::registry::BrainRecord) -> Value {
    let mut v = serde_json::to_value(rec).unwrap_or_else(|_| json!({}));
    if let Some(obj) = v.as_object_mut() {
        obj.insert("clone_path".into(), json!(format!("/git/{}.git", rec.config.name)));
        obj.insert("git_available".into(), json!(state.git.is_some()));
    }
    v
}

async fn list_handler(State(state): State<BrainState>) -> Response {
    match state.registry.list().await {
        Ok(brains) => Json(json!({
            "git_available": state.git.is_some(),
            "brains": brains.iter().map(|b| brain_json(&state, b)).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct CreateBody {
    name: String,
    /// Explicit realm list; omitted/empty + `all_realms: true` ⇒ all.
    #[serde(default)]
    realms: Vec<String>,
    #[serde(default)]
    all_realms: bool,
    #[serde(default)]
    include: Option<BrainFilters>,
    #[serde(default)]
    export_interval_secs: Option<u64>,
    #[serde(default)]
    gardener: Option<GardenerConfig>,
    /// Skip the immediate first export (default: run it).
    #[serde(default)]
    defer_first_export: bool,
}

async fn create_handler(
    State(state): State<BrainState>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<CreateBody>,
) -> Response {
    if let Some(r) = require_role(&principal, Role::Admin, "brain create") {
        return r;
    }
    let Some(git) = state.git.clone() else {
        return err(StatusCode::SERVICE_UNAVAILABLE, "git binary not found on server host");
    };
    let selector = if body.all_realms {
        RealmSelector::All
    } else if body.realms.is_empty() {
        return err(StatusCode::BAD_REQUEST, "provide realms or all_realms: true");
    } else {
        RealmSelector::Realms(body.realms.clone())
    };
    let now = Utc::now().to_rfc3339();
    let mut cfg = match BrainConfig::new(&body.name, selector, &now) {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_REQUEST, e.to_string()),
    };
    if let Some(f) = body.include {
        cfg.include = f;
    }
    if let Some(secs) = body.export_interval_secs {
        cfg.export_interval_secs = secs;
    }
    if let Some(g) = body.gardener {
        cfg.gardener = g;
    }
    match state.registry.get(&cfg.name).await {
        Ok(Some(_)) => return err(StatusCode::CONFLICT, format!("brain `{}` already exists", cfg.name)),
        Ok(None) => {}
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
    let repo = state.repo_dir(&cfg.name);
    if let Err(e) = git.init_bare(&repo).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, format!("init repo: {e}"));
    }
    if let Err(e) = state.registry.upsert_config(&cfg).await {
        return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string());
    }
    let export = if body.defer_first_export {
        json!({ "skipped": true })
    } else {
        match run_export_now(&state, &cfg).await {
            Ok(v) => v,
            Err(e) => json!({ "error": e }),
        }
    };
    match state.registry.get(&cfg.name).await {
        Ok(Some(rec)) => (
            StatusCode::CREATED,
            Json(json!({ "brain": brain_json(&state, &rec), "first_export": export })),
        )
            .into_response(),
        _ => (StatusCode::CREATED, Json(json!({ "first_export": export }))).into_response(),
    }
}

async fn get_handler(State(state): State<BrainState>, Path(name): Path<String>) -> Response {
    match state.registry.get(&name).await {
        Ok(Some(rec)) => Json(brain_json(&state, &rec)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, format!("brain `{name}` not found")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct UpdateBody {
    #[serde(default)]
    include: Option<BrainFilters>,
    #[serde(default)]
    export_interval_secs: Option<u64>,
    #[serde(default)]
    gardener: Option<GardenerConfig>,
    #[serde(default)]
    visibility_role: Option<String>,
}

async fn update_handler(
    State(state): State<BrainState>,
    Path(name): Path<String>,
    Extension(principal): Extension<Principal>,
    Json(body): Json<UpdateBody>,
) -> Response {
    if let Some(r) = require_role(&principal, Role::Admin, "brain update") {
        return r;
    }
    let mut rec = match state.registry.get(&name).await {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, format!("brain `{name}` not found")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    if let Some(f) = body.include {
        rec.config.include = f;
    }
    if let Some(secs) = body.export_interval_secs {
        rec.config.export_interval_secs = secs;
    }
    if let Some(g) = body.gardener {
        rec.config.gardener = g;
    }
    if let Some(v) = body.visibility_role {
        if !matches!(v.as_str(), "read" | "write" | "admin") {
            return err(StatusCode::BAD_REQUEST, "visibility_role must be read|write|admin");
        }
        rec.config.visibility_role = v;
    }
    rec.config.updated_at = Utc::now().to_rfc3339();
    match state.registry.upsert_config(&rec.config).await {
        Ok(()) => Json(brain_json(&state, &rec)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct DeleteParams {
    /// Also remove the bare repo directory (memories are never touched).
    #[serde(default)]
    purge: bool,
}

async fn delete_handler(
    State(state): State<BrainState>,
    Path(name): Path<String>,
    Extension(principal): Extension<Principal>,
    Query(params): Query<DeleteParams>,
) -> Response {
    if let Some(r) = require_role(&principal, Role::Admin, "brain delete") {
        return r;
    }
    if kyma_brain::validate_name(&name).is_err() {
        return err(StatusCode::BAD_REQUEST, "invalid brain name");
    }
    let lock = state.lock_for(&name);
    let _guard = lock.lock().await;
    match state.registry.delete(&name).await {
        Ok(()) => {}
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
    let mut purged = false;
    if params.purge {
        let repo = state.repo_dir(&name);
        if repo.exists() {
            purged = tokio::fs::remove_dir_all(&repo).await.is_ok();
        }
    }
    Json(json!({ "deleted": name, "repo_purged": purged, "note": "memories are not deleted" }))
        .into_response()
}

/// Run one export pass under the brain lock and record the run. Shared by
/// the create/export handlers, the schedulers, and the fabric executor.
pub async fn run_export_now(state: &BrainState, cfg: &BrainConfig) -> Result<Value, String> {
    let Some(git) = state.git.clone() else {
        return Err("git binary not found on server host".to_string());
    };
    let lock = state.lock_for(&cfg.name);
    let _guard = lock.lock().await;

    let started = Utc::now().to_rfc3339();
    let (nodes, edges) = fetch::fetch_rows(&state.agent, cfg).await?;
    let repo = state.repo_dir(&cfg.name);
    let outcome =
        kyma_brain::export::run_export(&git, &repo, cfg, &nodes, &edges, Utc::now().timestamp())
            .await;

    let mut rec = state
        .registry
        .get(&cfg.name)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("brain `{}` disappeared during export", cfg.name))?;
    let finished = Utc::now().to_rfc3339();

    match outcome {
        Ok(out) => {
            rec.runtime.last_export_at = Some(finished.clone());
            rec.runtime.last_commit = Some(out.commit.clone());
            rec.runtime.last_error = None;
            rec.runtime.note_count = out.note_count;
            rec.runtime.record_run(BrainRunRecord {
                kind: "export".into(),
                started_at: started,
                finished_at: finished,
                commit: Some(out.commit.clone()),
                files_written: out.files_written,
                files_deleted: 0,
                notes_ingested: 0,
                noop: out.noop,
                error: None,
                warnings: vec![],
            });
            state
                .registry
                .update_runtime(&cfg.name, &rec.runtime)
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({
                "commit": out.commit,
                "noop": out.noop,
                "notes": out.note_count,
                "files": out.files_written,
                "preserved": out.files_preserved,
            }))
        }
        Err(e) => {
            let msg = e.to_string();
            rec.runtime.last_error = Some(msg.clone());
            rec.runtime.record_run(BrainRunRecord {
                kind: "export".into(),
                started_at: started,
                finished_at: finished,
                commit: None,
                files_written: 0,
                files_deleted: 0,
                notes_ingested: 0,
                noop: false,
                error: Some(msg.clone()),
                warnings: vec![],
            });
            let _ = state.registry.update_runtime(&cfg.name, &rec.runtime).await;
            Err(msg)
        }
    }
}

async fn export_handler(
    State(state): State<BrainState>,
    Path(name): Path<String>,
    Extension(principal): Extension<Principal>,
) -> Response {
    if let Some(r) = require_role(&principal, Role::Write, "brain export") {
        return r;
    }
    let rec = match state.registry.get(&name).await {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, format!("brain `{name}` not found")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    match run_export_now(&state, &rec.config).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

/// Kick off a wiki-gardener run for this brain: a dreaming run whose focus
/// is [`kyma_brain::gardener::gardener_focus`]. Local mode runs it inline
/// (dreaming store guard dedupes); hosted mode enqueues a `dreaming` fabric
/// job. The gardener writes wiki memories; the next export renders them.
pub async fn trigger_gardener(state: &BrainState, cfg: &kyma_brain::registry::BrainConfig) -> Result<Value, String> {
    let focus = kyma_brain::gardener::gardener_focus(cfg);
    let result = if let Some(store) = state.agent.local_dreaming.clone() {
        if !store.try_acquire() {
            return Ok(json!({ "deduped": true, "detail": "a dreaming run is already in flight" }));
        }
        crate::agent::dreaming::spawn_local_run(
            state.agent.clone(),
            store,
            None,
            Some(focus),
            crate::agent::dreaming::Trigger::Manual,
        );
        json!({ "started": true, "mode": "local" })
    } else if let Some(pool) = state.agent.pool.clone() {
        let fabric = kyma_catalog::PgFabricStore::new(pool);
        let payload = serde_json::to_value(crate::agent::dreaming::DreamingRequest {
            trigger: crate::agent::dreaming::Trigger::Manual,
            mode: None,
            focus: Some(focus),
            job_id: None,
            worker_id: None,
        })
        .map_err(|e| e.to_string())?;
        let enqueued = fabric
            .enqueue_job(
                state.agent.tenant,
                &kyma_core::fabric::EnqueueJob {
                    kind: kyma_core::fabric::JOB_DREAMING.to_string(),
                    payload,
                    priority: 5,
                    affinity_worker_id: None,
                    req_capabilities: vec!["dreaming".into()],
                    label_selector: json!({}),
                    max_attempts: 1,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        match enqueued {
            Some(job_id) => json!({ "started": true, "mode": "fabric", "job_id": job_id }),
            None => json!({ "deduped": true, "detail": "a dreaming run is already in flight" }),
        }
    } else {
        return Err("gardener requires the local dreaming store or Postgres".to_string());
    };
    // Stamp last_gardener_at so the scheduler's interval check advances even
    // when the run itself is recorded in the dreaming surface.
    if let Ok(Some(mut rec)) = state.registry.get(&cfg.name).await {
        rec.runtime.last_gardener_at = Some(Utc::now().to_rfc3339());
        let _ = state.registry.update_runtime(&cfg.name, &rec.runtime).await;
    }
    Ok(result)
}

async fn garden_handler(
    State(state): State<BrainState>,
    Path(name): Path<String>,
    Extension(principal): Extension<Principal>,
) -> Response {
    if let Some(r) = require_role(&principal, Role::Write, "brain garden") {
        return r;
    }
    let rec = match state.registry.get(&name).await {
        Ok(Some(r)) => r,
        Ok(None) => return err(StatusCode::NOT_FOUND, format!("brain `{name}` not found")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    match trigger_gardener(&state, &rec.config).await {
        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn runs_handler(State(state): State<BrainState>, Path(name): Path<String>) -> Response {
    match state.registry.get(&name).await {
        Ok(Some(rec)) => Json(json!({ "runs": rec.runtime.runs })).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, format!("brain `{name}` not found")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

/// Resolve a brain to (git, repo dir, HEAD sha) for the read-only browse
/// endpoints. 404s cover unknown brains and never-exported repos alike.
async fn browse_repo(
    state: &BrainState,
    name: &str,
) -> Result<(std::sync::Arc<kyma_brain::gitbin::GitBin>, std::path::PathBuf, String), Response> {
    let Some(git) = state.git.clone() else {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "git binary not found on server host"));
    };
    match state.registry.get(name).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err(err(StatusCode::NOT_FOUND, format!("brain `{name}` not found"))),
        Err(e) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
    let dir = state.repo_dir(name);
    let head = match git.rev_parse(&dir, &format!("refs/heads/{}", kyma_brain::BRAIN_BRANCH)).await
    {
        Ok(Some(h)) => h,
        Ok(None) => return Err(err(StatusCode::NOT_FOUND, "brain has no exports yet")),
        Err(e) => return Err(err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };
    Ok((git, dir, head))
}

/// `GET /v1/brain/:name/tree` — every path in the vault at HEAD, for the
/// web UI's Obsidian-style browser. Titles are derived client-side from
/// filenames (no per-blob reads — trees stay O(1) git calls).
async fn tree_handler(State(state): State<BrainState>, Path(name): Path<String>) -> Response {
    let (git, dir, head) = match browse_repo(&state, &name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match git.ls_tree_paths(&dir, &head).await {
        Ok(paths) => Json(json!({ "head": head, "paths": paths })).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct FileParams {
    path: String,
}

/// Max blob size served through the browse endpoint (the UI renders notes,
/// not packfiles).
const MAX_BROWSE_BYTES: usize = 2 * 1024 * 1024;

/// `GET /v1/brain/:name/file?path=…` — one blob at HEAD as JSON
/// `{path, head, content}` (UTF-8 only; binary → 415).
async fn file_handler(
    State(state): State<BrainState>,
    Path(name): Path<String>,
    Query(params): Query<FileParams>,
) -> Response {
    let path = params.path;
    if path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return err(StatusCode::BAD_REQUEST, "invalid path");
    }
    let (git, dir, head) = match browse_repo(&state, &name).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match git.cat_file(&dir, &head, &path).await {
        Ok(bytes) if bytes.len() > MAX_BROWSE_BYTES => {
            err(StatusCode::PAYLOAD_TOO_LARGE, "file too large to preview")
        }
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(content) => Json(json!({ "path": path, "head": head, "content": content }))
                .into_response(),
            Err(_) => err(StatusCode::UNSUPPORTED_MEDIA_TYPE, "binary file"),
        },
        Err(_) => err(StatusCode::NOT_FOUND, format!("no file at {path}")),
    }
}

async fn clone_info_handler(State(state): State<BrainState>, Path(name): Path<String>) -> Response {
    match state.registry.get(&name).await {
        Ok(Some(rec)) => Json(json!({
            "path": format!("/git/{}.git", rec.config.name),
            "auth": "http basic — username anything (e.g. `kyma`), password = a kyma API token",
            "min_role": rec.config.visibility_role,
            "example": format!("git clone http://<host>:<port>/git/{}.git", rec.config.name),
        }))
        .into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, format!("brain `{name}` not found")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}
