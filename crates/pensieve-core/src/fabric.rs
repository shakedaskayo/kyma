//! Worker/job fabric wire types — the distributed context-engine control
//! plane vocabulary shared by the catalog store (`pensieve-catalog::fabric`), the
//! HTTP handlers (`pensieve-server::fabric_handler`), and fabric clients (the
//! embedded worker in pensieve-bin and the remote daemon in pensieve-local).
//!
//! A *worker* is a fabric compute identity: the server's embedded in-process
//! worker, or a remote daemon registered from any compute. A *job* is a
//! placement-aware unit of work a worker claims under a lease: data source
//! syncs, dreaming runs, or node-local source syncs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use uuid::Uuid;

/// Job kinds carried by the fabric in v1.
pub const JOB_DATA_SOURCE_SYNC: &str = "data_source_sync";
pub const JOB_DREAMING: &str = "dreaming";
pub const JOB_SOURCE_SYNC: &str = "source_sync";
/// Build index sidecars (ANN/FTS) for a set of extents — see
/// `pensieve-jobs::index_build`.
pub const JOB_INDEX_BUILD: &str = "index_build";
/// Periodic ANN sidecar maintenance (re-train / re-balance). S1 executor.
pub const JOB_ANN_MAINTAIN: &str = "ann_maintain";
/// Backfill embeddings for rows ingested before a vector column/model was
/// configured. S1 executor.
pub const JOB_EMBED_BACKFILL: &str = "embed_backfill";
/// Export one brain repo (memory → Obsidian-vault git commit). Payload
/// `{ "brain": <name> }` — see `pensieve-server::brain::executor`.
pub const JOB_BRAIN_EXPORT: &str = "brain_export";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    Embedded,
    Daemon,
}

impl WorkerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkerKind::Embedded => "embedded",
            WorkerKind::Daemon => "daemon",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerStatus {
    Online,
    Draining,
    Offline,
}

impl WorkerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkerStatus::Online => "online",
            WorkerStatus::Draining => "draining",
            WorkerStatus::Offline => "offline",
        }
    }
}

/// Registration payload a worker sends when connecting (idempotent on
/// (tenant, name): re-registering updates capabilities/sources/version).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRegistration {
    pub name: String,
    pub kind: WorkerKind,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default = "default_json_obj")]
    pub labels: Json,
    #[serde(default = "default_json_obj")]
    pub sources: Json,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: i32,
    #[serde(default)]
    pub version: Option<String>,
}

fn default_json_obj() -> Json {
    Json::Object(Default::default())
}

fn default_max_concurrent() -> i32 {
    1
}

/// An active coding-agent session a worker observes locally (relayed via the
/// node daemon's hooks relay) — the fabric's presence primitive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceSession {
    pub session_id: String,
    #[serde(default)]
    pub realm: Option<String>,
    /// The coding-agent kind this session belongs to (e.g. `"claude-code"`).
    /// `None` on older nodes that predate per-agent presence (wire-compatible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub last_activity: DateTime<Utc>,
}

/// Worker heartbeat payload. The worker identity comes from auth (token or
/// in-process), never from the body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Heartbeat {
    #[serde(default)]
    pub current_jobs: Vec<Uuid>,
    #[serde(default)]
    pub presence: Vec<PresenceSession>,
    /// Refreshed source inventory; `None` leaves the stored value untouched.
    #[serde(default)]
    pub sources: Option<Json>,
}

/// Operator/discovery view of a worker (`GET /v1/workers`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerView {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub hostname: Option<String>,
    pub kind: WorkerKind,
    pub capabilities: Vec<String>,
    pub labels: Json,
    pub sources: Json,
    pub status: WorkerStatus,
    pub max_concurrent: i32,
    #[serde(default)]
    pub version: Option<String>,
    pub presence: Vec<PresenceSession>,
    #[serde(default)]
    pub last_heartbeat: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Enqueue request for a new job (scheduler- or operator-initiated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnqueueJob {
    pub kind: String,
    #[serde(default = "default_json_obj")]
    pub payload: Json,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub affinity_worker_id: Option<Uuid>,
    #[serde(default)]
    pub req_capabilities: Vec<String>,
    #[serde(default = "default_json_obj")]
    pub label_selector: Json,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: i32,
}

fn default_max_attempts() -> i32 {
    3
}

/// The enqueue side of the worker fabric, abstracted so background schedulers
/// (index build, embed backfill, FTS) can target either the Postgres fabric
/// queue (server mode) or the local SQLite `jobs` table (embedded mode) without
/// knowing which. The *claim* side stays catalog-specific (`PgFabricStore` /
/// `SqliteQueue`); only enqueue needs to be portable for the schedulers to run
/// in local mode. The method is named `enqueue` (not `enqueue_job`) so impls on
/// types that already have an inherent `enqueue_job` — like `PgFabricStore` —
/// don't shadow it.
#[async_trait::async_trait]
pub trait JobEnqueuer: Send + Sync {
    async fn enqueue(
        &self,
        tenant: crate::tenant::TenantId,
        job: &EnqueueJob,
    ) -> crate::errors::Result<Option<Uuid>>;
}

/// A job handed to a worker by a successful claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimedJob {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub kind: String,
    pub payload: Json,
    pub attempt: i32,
    pub max_attempts: i32,
    pub claim_expires_at: DateTime<Utc>,
    /// Resolved secrets for remote workers (delivered over TLS in the claim
    /// response; embedded claims leave this `None` and use the in-process
    /// credential store).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Json>,
}

/// Job status as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Claimed,
    Running,
    Done,
    Failed,
    Canceled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Claimed => "claimed",
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
            JobStatus::Canceled => "canceled",
        }
    }
}

/// Operator view of a job (`GET /v1/jobs`, `GET /v1/jobs/{id}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobView {
    pub id: Uuid,
    pub kind: String,
    pub status: JobStatus,
    pub payload: Json,
    pub priority: i32,
    #[serde(default)]
    pub affinity_worker_id: Option<Uuid>,
    pub req_capabilities: Vec<String>,
    #[serde(default)]
    pub claimed_by: Option<Uuid>,
    #[serde(default)]
    pub claim_expires_at: Option<DateTime<Utc>>,
    pub attempt: i32,
    pub max_attempts: i32,
    pub progress: Json,
    #[serde(default)]
    pub result: Option<Json>,
    #[serde(default)]
    pub dreaming_run_id: Option<Uuid>,
    #[serde(default)]
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
