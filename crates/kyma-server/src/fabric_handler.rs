//! HTTP control plane for the worker/job fabric (`/v1/workers`, `/v1/jobs`).
//!
//! Two routers with different auth:
//! - [`worker_router`] — worker-token (`kyw_…`) authenticated endpoints remote
//!   daemons use: register, heartbeat, claim (long-poll), progress, complete,
//!   fail, lease. The worker identity always comes from the token, never the
//!   body.
//! - [`admin_router`] — operator endpoints behind the regular bearer
//!   middleware: issue/revoke worker tokens, node discovery, manual job
//!   enqueue, and the operator job views.
//!
//! The server's embedded worker bypasses HTTP entirely and talks to
//! [`PgFabricStore`] in-process.

use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use futures::future::BoxFuture;
use kyma_catalog::{PgFabricStore, WorkerAuth};
use kyma_core::fabric::{
    ClaimedJob, EnqueueJob, Heartbeat, WorkerRegistration, WorkerStatus,
};
use kyma_core::tenant::TenantId;
use serde::Deserialize;
use serde_json::{json, Value as Json_};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use uuid::Uuid;

/// Optional claim-time enrichment: resolve per-job secrets/runspecs for
/// REMOTE workers (e.g. a datasource_sync claim carries the decrypted
/// credential so the daemon never needs `KYMA_SECRET_KEY`). Wired by kyma-bin.
pub type ClaimEnricher =
    Arc<dyn Fn(ClaimedJob) -> BoxFuture<'static, anyhow::Result<ClaimedJob>> + Send + Sync>;

#[derive(Clone)]
pub struct FabricState {
    pub store: Arc<PgFabricStore>,
    pub enricher: Option<ClaimEnricher>,
    /// Caps concurrent long-poll claim requests so idle daemons can't pin the
    /// whole connection pool; excess claims degrade to a single attempt.
    pub longpoll: Arc<Semaphore>,
    pub lease_secs: i64,
}

impl FabricState {
    pub fn new(store: Arc<PgFabricStore>, enricher: Option<ClaimEnricher>) -> Self {
        let lease_secs = std::env::var("KYMA_FABRIC_LEASE_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300);
        Self {
            store,
            enricher,
            longpoll: Arc::new(Semaphore::new(32)),
            lease_secs,
        }
    }
}

/// `kyw_` + 256 bits of CSPRNG entropy, hex-encoded. Shown exactly once.
pub fn mint_worker_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("kyw_{}", hex::encode(bytes))
}

/// Worker tokens carry ≥128 bits of CSPRNG entropy — raw SHA-256 (no salt)
/// is the right lookup hash, same as session tokens.
pub fn worker_token_hash(token: &str) -> String {
    hex::encode(crate::auth::hash_token(token))
}

// ── worker-token middleware ──────────────────────────────────────────────────

pub async fn require_worker_middleware(
    State(state): State<FabricState>,
    mut req: Request,
    next: Next,
) -> Response {
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim);
    let Some(token) = token else {
        return unauthorized("missing Authorization: Bearer <worker token>");
    };
    if !token.starts_with("kyw_") {
        return unauthorized("not a worker token");
    }
    let auth = match state.store.authenticate_worker(&worker_token_hash(token)).await {
        Ok(Some(auth)) => auth,
        Ok(None) => return unauthorized("unknown or revoked worker token"),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("worker auth: {e}"),
            )
                .into_response()
        }
    };
    req.extensions_mut().insert(auth);
    next.run(req).await
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, r#"Bearer realm="kyma-fabric""#)],
        msg.to_owned(),
    )
        .into_response()
}

// ── worker-facing router ─────────────────────────────────────────────────────

pub fn worker_router(state: FabricState) -> Router {
    Router::new()
        .route("/v1/workers/register", post(register))
        .route("/v1/workers/heartbeat", post(heartbeat))
        .route("/v1/jobs/claim", post(claim))
        .route("/v1/jobs/self", post(enqueue_self))
        .route("/v1/jobs/:id/progress", post(job_progress))
        .route("/v1/jobs/:id/complete", post(job_complete))
        .route("/v1/jobs/:id/fail", post(job_fail))
        .route("/v1/jobs/:id/lease", post(job_lease))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_worker_middleware,
        ))
        .with_state(state)
}

async fn register(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Json(reg): Json<WorkerRegistration>,
) -> impl IntoResponse {
    match s.store.register_worker(auth.worker_id, &reg).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({
                "worker_id": auth.worker_id,
                "heartbeat_interval_secs": 30,
                "lease_secs": s.lease_secs,
            })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

async fn heartbeat(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Json(hb): Json<Heartbeat>,
) -> impl IntoResponse {
    match s.store.touch_heartbeat(auth.worker_id, &hb).await {
        Ok(status) => (
            StatusCode::OK,
            Json(json!({
                "status": status,
                "drain": status == WorkerStatus::Draining,
                "recommended_poll_ms": 1000,
            })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct ClaimReq {
    /// Job kinds this worker executes (its registered executor set).
    kinds: Vec<String>,
    #[serde(default = "default_claim_max")]
    max: usize,
    /// Long-poll budget; the server holds the request up to this long waiting
    /// for work (capped at 25s). 0 = single immediate attempt.
    #[serde(default)]
    wait_ms: u64,
}

fn default_claim_max() -> usize {
    1
}

async fn claim(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Json(req): Json<ClaimReq>,
) -> impl IntoResponse {
    if req.kinds.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "kinds must be non-empty" })),
        )
            .into_response();
    }
    let max = req.max.clamp(1, 16);
    // Long-poll only while a permit is available; otherwise degrade to one
    // immediate attempt so claim never blocks on the semaphore itself.
    let permit = s.longpoll.clone().try_acquire_owned().ok();
    let wait = if permit.is_some() {
        Duration::from_millis(req.wait_ms.min(25_000))
    } else {
        Duration::ZERO
    };
    let deadline = tokio::time::Instant::now() + wait;

    let mut jobs: Vec<ClaimedJob> = Vec::new();
    loop {
        while jobs.len() < max {
            match s
                .store
                .claim_job(
                    auth.worker_id,
                    Some(auth.tenant),
                    &auth.capabilities,
                    &req.kinds,
                    s.lease_secs,
                )
                .await
            {
                Ok(Some(job)) => {
                    let job = match &s.enricher {
                        Some(enrich) => match enrich(job).await {
                            Ok(j) => j,
                            Err(e) => return internal(e),
                        },
                        None => job,
                    };
                    jobs.push(job);
                }
                Ok(None) => break,
                Err(e) => return internal(e),
            }
        }
        if !jobs.is_empty() || tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    drop(permit);
    (StatusCode::OK, Json(json!({ "jobs": jobs }))).into_response()
}

#[derive(Deserialize)]
struct SelfEnqueueReq {
    kind: String,
    #[serde(default)]
    payload: Json_,
    #[serde(default)]
    priority: i32,
}

/// Worker self-enqueue: a node schedules work only IT can perform (e.g.
/// `source_sync` over its local files). Always pinned to the caller — a
/// worker token cannot enqueue work for other workers or unpinned kinds.
async fn enqueue_self(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Json(req): Json<SelfEnqueueReq>,
) -> impl IntoResponse {
    let payload = if req.payload.is_null() {
        json!({})
    } else {
        req.payload
    };
    match s
        .store
        .enqueue_job(
            auth.tenant,
            &EnqueueJob {
                kind: req.kind,
                payload,
                priority: req.priority,
                affinity_worker_id: Some(auth.worker_id),
                req_capabilities: vec![],
                label_selector: json!({}),
                max_attempts: 3,
            },
        )
        .await
    {
        Ok(Some(id)) => (StatusCode::CREATED, Json(json!({ "job_id": id }))).into_response(),
        Ok(None) => (
            StatusCode::OK,
            Json(json!({ "job_id": Json_::Null, "deduped": true })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

async fn job_progress(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Path(id): Path<Uuid>,
    Json(snapshot): Json<Json_>,
) -> impl IntoResponse {
    match s.store.job_progress(id, auth.worker_id, snapshot).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => lease_lost(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct CompleteReq {
    #[serde(default)]
    result: Json_,
}

async fn job_complete(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Path(id): Path<Uuid>,
    Json(req): Json<CompleteReq>,
) -> impl IntoResponse {
    match s.store.complete_job(id, auth.worker_id, req.result).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => lease_lost(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct FailReq {
    error: String,
}

async fn job_fail(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Path(id): Path<Uuid>,
    Json(req): Json<FailReq>,
) -> impl IntoResponse {
    match s.store.fail_job(id, auth.worker_id, &req.error).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => lease_lost(),
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct LeaseReq {
    #[serde(default)]
    lease_secs: Option<i64>,
}

async fn job_lease(
    Extension(auth): Extension<WorkerAuth>,
    State(s): State<FabricState>,
    Path(id): Path<Uuid>,
    Json(req): Json<LeaseReq>,
) -> impl IntoResponse {
    let lease = req.lease_secs.unwrap_or(s.lease_secs).clamp(10, 3600);
    match s.store.extend_lease(id, auth.worker_id, lease).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => lease_lost(),
        Err(e) => internal(e),
    }
}

fn lease_lost() -> Response {
    (
        StatusCode::CONFLICT,
        Json(json!({ "error": "job is not held by this worker (lease lost or terminal)" })),
    )
        .into_response()
}

// ── admin/operator router ────────────────────────────────────────────────────

/// Mounted behind the regular bearer middleware (Role::Write) by kyma-bin.
pub fn admin_router(state: FabricState) -> Router {
    Router::new()
        .route("/v1/workers", post(create_worker).get(list_workers))
        .route("/v1/workers/:id", axum::routing::delete(revoke_worker))
        .route("/v1/jobs", post(enqueue_job).get(list_jobs))
        .route("/v1/jobs/:id", get(get_job))
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateWorkerReq {
    name: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    labels: Json_,
}

async fn create_worker(
    Extension(tenant): Extension<TenantId>,
    State(s): State<FabricState>,
    Json(req): Json<CreateWorkerReq>,
) -> impl IntoResponse {
    let name = req.name.trim();
    if name.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "name is required" })),
        )
            .into_response();
    }
    let labels = if req.labels.is_null() {
        json!({})
    } else {
        req.labels
    };
    let token = mint_worker_token();
    match s
        .store
        .create_worker(tenant, name, &worker_token_hash(&token), &req.capabilities, labels)
        .await
    {
        Ok(worker_id) => (
            StatusCode::CREATED,
            Json(json!({
                "worker_id": worker_id,
                // Shown exactly once; only the hash is stored.
                "token": token,
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn list_workers(
    Extension(tenant): Extension<TenantId>,
    State(s): State<FabricState>,
) -> impl IntoResponse {
    match s.store.list_workers(tenant).await {
        Ok(items) => (StatusCode::OK, Json(json!({ "items": items }))).into_response(),
        Err(e) => internal(e),
    }
}

async fn revoke_worker(
    Extension(tenant): Extension<TenantId>,
    State(s): State<FabricState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.store.revoke_worker(tenant, id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such worker" })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

async fn enqueue_job(
    Extension(tenant): Extension<TenantId>,
    State(s): State<FabricState>,
    Json(req): Json<EnqueueJob>,
) -> impl IntoResponse {
    match s.store.enqueue_job(tenant, &req).await {
        Ok(Some(id)) => (
            StatusCode::CREATED,
            Json(json!({ "job_id": id })),
        )
            .into_response(),
        // Idempotency index skipped the insert — an equivalent job is in flight.
        Ok(None) => (
            StatusCode::OK,
            Json(json!({ "job_id": Json_::Null, "deduped": true })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct ListJobsQuery {
    kind: Option<String>,
    status: Option<String>,
    #[serde(default = "default_jobs_limit")]
    limit: i64,
}

fn default_jobs_limit() -> i64 {
    50
}

async fn list_jobs(
    Extension(tenant): Extension<TenantId>,
    State(s): State<FabricState>,
    Query(q): Query<ListJobsQuery>,
) -> impl IntoResponse {
    match s
        .store
        .list_jobs(tenant, q.kind.as_deref(), q.status.as_deref(), q.limit.clamp(1, 500))
        .await
    {
        Ok(items) => (StatusCode::OK, Json(json!({ "items": items }))).into_response(),
        Err(e) => internal(e),
    }
}

async fn get_job(
    Extension(tenant): Extension<TenantId>,
    State(s): State<FabricState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match s.store.get_job(tenant, id).await {
        Ok(Some(job)) => (StatusCode::OK, Json(job)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such job" })),
        )
            .into_response(),
        Err(e) => internal(e),
    }
}

fn internal(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": e.to_string() })),
    )
        .into_response()
}
