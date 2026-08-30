//! The worker run loop: claim → dispatch → report, with lease keep-alive.

use crate::executor::{JobCtx, JobError, JobExecutor, ProgressSink};
use crate::queue::JobQueue;
use kyma_core::fabric::ClaimedJob;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// Job-kind → executor dispatch table.
#[derive(Default, Clone)]
pub struct ExecutorRegistry {
    by_kind: HashMap<&'static str, Arc<dyn JobExecutor>>,
}

impl ExecutorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, exec: Arc<dyn JobExecutor>) {
        self.by_kind.insert(exec.kind(), exec);
    }

    pub fn kinds(&self) -> Vec<String> {
        self.by_kind.keys().map(|k| (*k).to_string()).collect()
    }

    fn lookup(&self, kind: &str) -> Option<Arc<dyn JobExecutor>> {
        self.by_kind.get(kind).cloned()
    }
}

/// One worker loop. Spawn N of these for N-way concurrency — they share the
/// queue front-end and the registry; the claim query keeps them from
/// stepping on each other.
pub struct JobRunner {
    pub queue: Arc<dyn JobQueue>,
    pub executors: ExecutorRegistry,
    pub idle_sleep: Duration,
    /// Lease keep-alive cadence while an executor runs (≈ half the lease).
    pub lease_keepalive: Duration,
    pub lease_secs: i64,
}

impl JobRunner {
    pub fn new(queue: Arc<dyn JobQueue>, executors: ExecutorRegistry, lease_secs: i64) -> Self {
        Self {
            queue,
            executors,
            idle_sleep: Duration::from_millis(500),
            lease_keepalive: Duration::from_secs((lease_secs as u64 / 2).max(5)),
            lease_secs,
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        let kinds = self.executors.kinds();
        info!(kinds = ?kinds, "job runner starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("job runner shutdown"); return; }
                res = self.claim_and_run_one(&kinds) => match res {
                    Ok(true) => {}
                    Ok(false) => tokio::time::sleep(self.idle_sleep).await,
                    Err(e) => {
                        error!(error = %e, "job runner error");
                        tokio::time::sleep(self.idle_sleep).await;
                    }
                }
            }
        }
    }

    pub async fn claim_and_run_one(&self, kinds: &[String]) -> anyhow::Result<bool> {
        let jobs = self.queue.claim(kinds, 1).await?;
        let Some(job) = jobs.into_iter().next() else {
            return Ok(false);
        };
        self.run_job(job).await;
        Ok(true)
    }

    async fn run_job(&self, job: ClaimedJob) {
        let Some(exec) = self.executors.lookup(&job.kind) else {
            // Should not happen — we only claim registered kinds.
            warn!(job_id = %job.id, kind = %job.kind, "no executor for claimed job");
            let _ = self
                .queue
                .fail_terminal(job.id, &format!("no executor for kind {}", job.kind))
                .await;
            return;
        };
        let ctx = JobCtx {
            queue: self.queue.clone(),
            progress: ProgressSink::new(self.queue.clone(), job.id),
        };
        info!(job_id = %job.id, kind = %job.kind, attempt = job.attempt, "job start");

        // Drive the executor with a lease keep-alive heartbeat so long jobs
        // (a big repo sync, a dreaming run) outlive the base lease.
        let exec_fut = exec.run(&ctx, &job);
        tokio::pin!(exec_fut);
        let mut keepalive = tokio::time::interval(self.lease_keepalive);
        keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        keepalive.tick().await; // first tick fires immediately — consume it.
        let outcome = loop {
            tokio::select! {
                out = &mut exec_fut => break out,
                _ = keepalive.tick() => {
                    match self.queue.extend_lease(job.id, self.lease_secs).await {
                        Ok(true) => {}
                        Ok(false) => warn!(job_id = %job.id, "lease lost while running"),
                        Err(e) => warn!(job_id = %job.id, error = %e, "lease extend failed"),
                    }
                }
            }
        };

        match outcome {
            Ok(result) => {
                info!(job_id = %job.id, kind = %job.kind, "job done");
                if let Err(e) = self.queue.complete(job.id, result).await {
                    error!(job_id = %job.id, error = %e, "complete failed");
                }
            }
            Err(JobError::Transient(msg)) => {
                warn!(job_id = %job.id, kind = %job.kind, error = %msg, "job transient failure");
                if let Err(e) = self.queue.fail(job.id, &msg).await {
                    error!(job_id = %job.id, error = %e, "fail report failed");
                }
            }
            Err(err @ (JobError::Permanent(_) | JobError::Config(_))) => {
                let msg = err.to_string();
                warn!(job_id = %job.id, kind = %job.kind, error = %msg, "job terminal failure");
                if let Err(e) = self.queue.fail_terminal(job.id, &msg).await {
                    error!(job_id = %job.id, error = %e, "terminal fail report failed");
                }
            }
        }
    }
}
