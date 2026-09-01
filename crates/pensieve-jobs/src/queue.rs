//! Queue front-ends: how a worker reaches the control plane.

use async_trait::async_trait;
use pensieve_catalog::PgFabricStore;
use pensieve_core::fabric::ClaimedJob;
use pensieve_core::tenant::TenantId;
use serde_json::Value as Json;
use std::sync::Arc;
use uuid::Uuid;

/// Worker-side view of the control plane. The embedded worker implements it
/// with direct Postgres ([`PgQueue`]); remote daemons implement it over the
/// `/v1/jobs` HTTP API with their worker token.
#[async_trait]
pub trait JobQueue: Send + Sync {
    /// Claim up to `max` jobs of the given kinds. Implementations apply the
    /// worker's capabilities and tenant scope.
    async fn claim(&self, kinds: &[String], max: usize) -> anyhow::Result<Vec<ClaimedJob>>;
    /// Replace the live progress snapshot. `false` = lease lost.
    async fn progress(&self, job_id: Uuid, snapshot: Json) -> anyhow::Result<bool>;
    async fn complete(&self, job_id: Uuid, result: Json) -> anyhow::Result<bool>;
    /// Retryable failure (re-queued while attempts remain).
    async fn fail(&self, job_id: Uuid, error: &str) -> anyhow::Result<bool>;
    /// Terminal failure (no retry).
    async fn fail_terminal(&self, job_id: Uuid, error: &str) -> anyhow::Result<bool>;
    async fn extend_lease(&self, job_id: Uuid, lease_secs: i64) -> anyhow::Result<bool>;
    /// Link a dreaming run record to its job.
    async fn link_dreaming_run(&self, job_id: Uuid, run_id: Uuid) -> anyhow::Result<()>;
}

/// In-process queue front-end for the server's embedded worker.
pub struct PgQueue {
    store: Arc<PgFabricStore>,
    worker_id: Uuid,
    /// `None` = all-tenant claim (the embedded worker serves every tenant on
    /// this deployment); remote daemons are always tenant-scoped by token.
    tenant: Option<TenantId>,
    capabilities: Vec<String>,
    lease_secs: i64,
}

impl PgQueue {
    pub fn new(
        store: Arc<PgFabricStore>,
        worker_id: Uuid,
        tenant: Option<TenantId>,
        capabilities: Vec<String>,
        lease_secs: i64,
    ) -> Self {
        Self {
            store,
            worker_id,
            tenant,
            capabilities,
            lease_secs,
        }
    }

    pub fn worker_id(&self) -> Uuid {
        self.worker_id
    }
}

#[async_trait]
impl JobQueue for PgQueue {
    async fn claim(&self, kinds: &[String], max: usize) -> anyhow::Result<Vec<ClaimedJob>> {
        let mut out = Vec::new();
        for _ in 0..max {
            match self
                .store
                .claim_job(
                    self.worker_id,
                    self.tenant,
                    &self.capabilities,
                    kinds,
                    self.lease_secs,
                )
                .await?
            {
                Some(job) => out.push(job),
                None => break,
            }
        }
        Ok(out)
    }

    async fn progress(&self, job_id: Uuid, snapshot: Json) -> anyhow::Result<bool> {
        self.store.job_progress(job_id, self.worker_id, snapshot).await
    }

    async fn complete(&self, job_id: Uuid, result: Json) -> anyhow::Result<bool> {
        self.store.complete_job(job_id, self.worker_id, result).await
    }

    async fn fail(&self, job_id: Uuid, error: &str) -> anyhow::Result<bool> {
        self.store.fail_job(job_id, self.worker_id, error).await
    }

    async fn fail_terminal(&self, job_id: Uuid, error: &str) -> anyhow::Result<bool> {
        self.store
            .fail_job_terminal(job_id, self.worker_id, error)
            .await
    }

    async fn extend_lease(&self, job_id: Uuid, lease_secs: i64) -> anyhow::Result<bool> {
        self.store
            .extend_lease(job_id, self.worker_id, lease_secs)
            .await
    }

    async fn link_dreaming_run(&self, job_id: Uuid, run_id: Uuid) -> anyhow::Result<()> {
        self.store.link_dreaming_run(job_id, run_id).await
    }
}
