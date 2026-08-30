//! The per-kind job executor contract.

use crate::queue::JobQueue;
use async_trait::async_trait;
use pensieve_core::fabric::ClaimedJob;
use serde_json::Value as Json;
use std::sync::Arc;
use uuid::Uuid;

/// Executor failure, mapped onto queue semantics by the runner:
/// Transient → retryable fail; Permanent/Config → terminal fail.
#[derive(Debug)]
pub enum JobError {
    Transient(String),
    Permanent(String),
    Config(String),
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobError::Transient(m) => write!(f, "transient: {m}"),
            JobError::Permanent(m) => write!(f, "permanent: {m}"),
            JobError::Config(m) => write!(f, "config: {m}"),
        }
    }
}

/// Best-effort live progress writer for one job. Executors push whole
/// snapshots ({current_phase, pct, activity ring, counters}); failures are
/// logged, never fatal — progress is observability, not state.
#[derive(Clone)]
pub struct ProgressSink {
    queue: Arc<dyn JobQueue>,
    job_id: Uuid,
}

impl ProgressSink {
    pub fn new(queue: Arc<dyn JobQueue>, job_id: Uuid) -> Self {
        Self { queue, job_id }
    }

    pub async fn push(&self, snapshot: Json) {
        if let Err(e) = self.queue.progress(self.job_id, snapshot).await {
            tracing::debug!(job_id = %self.job_id, error = %e, "progress push failed");
        }
    }
}

/// Everything an executor gets for one job run.
pub struct JobCtx {
    pub queue: Arc<dyn JobQueue>,
    pub progress: ProgressSink,
}

#[async_trait]
pub trait JobExecutor: Send + Sync {
    /// The job kind this executor handles (e.g. `datasource_sync`).
    fn kind(&self) -> &'static str;
    /// Run one job to completion. The returned JSON becomes `jobs.result`.
    async fn run(&self, ctx: &JobCtx, job: &ClaimedJob) -> Result<Json, JobError>;
}
