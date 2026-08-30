//! `datasource_sync` executor — the data source tick re-hosted on the fabric.
//!
//! The tick body (load row + cursor → run data source → sink rows → graphs →
//! cursor + run status) is `pensieve_datasources::runner::run_data_source_tick`,
//! shared with the legacy background_tasks front-end. This executor only maps
//! the job payload in and the [`TickOutcome`] onto fabric job semantics.

use crate::executor::{JobCtx, JobError, JobExecutor};
use async_trait::async_trait;
use pensieve_datasources::runner::{run_data_source_tick, DataSourceTickDeps, TickOutcome};
use pensieve_core::fabric::ClaimedJob;
use pensieve_core::tenant::{TenantId, DEFAULT_TENANT};
use serde_json::{json, Value as Json};
use uuid::Uuid;

pub struct DataSourceSyncExecutor {
    deps: DataSourceTickDeps,
}

impl DataSourceSyncExecutor {
    pub fn new(deps: DataSourceTickDeps) -> Self {
        Self { deps }
    }
}

#[async_trait]
impl JobExecutor for DataSourceSyncExecutor {
    fn kind(&self) -> &'static str {
        pensieve_core::fabric::JOB_DATA_SOURCE_SYNC
    }

    async fn run(&self, ctx: &JobCtx, job: &ClaimedJob) -> Result<Json, JobError> {
        let data_source_id = job
            .payload
            .get("data_source_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| JobError::Config("job missing data_source_id".into()))?;
        let scheduled_for_ms = job
            .payload
            .get("scheduled_for")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| JobError::Config("job missing scheduled_for".into()))?;
        // The scheduler stamps tenant_id into the payload (and onto the jobs
        // row); older enqueues fall back to DEFAULT_TENANT.
        let tenant: TenantId = job
            .payload
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_TENANT);

        ctx.progress
            .push(json!({
                "current_phase": "syncing",
                "data_source_id": data_source_id,
            }))
            .await;

        let outcome = run_data_source_tick(&self.deps, tenant, data_source_id, scheduled_for_ms)
            .await
            .map_err(|e| JobError::Transient(e.to_string()))?;

        match outcome {
            TickOutcome::Ok { rows } => Ok(json!({ "rows": rows })),
            TickOutcome::Skipped(reason) => Ok(json!({ "skipped": reason })),
            // Retryable: the fabric re-queues within the attempt budget.
            TickOutcome::Transient(msg) => Err(JobError::Transient(msg)),
            // Recorded on the data source row; the job itself is done (mirrors
            // the background_tasks behavior of completing the task).
            TickOutcome::Permanent(msg) => Ok(json!({ "error": msg, "permanent": true })),
            TickOutcome::Config(msg) => Ok(json!({ "error": msg, "disabled": true })),
        }
    }
}
