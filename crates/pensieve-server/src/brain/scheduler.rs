//! Recurring brain exports.
//!
//! Hosted mode: [`BrainScheduler`] enqueues one `brain_export` fabric job
//! per due brain (executed by [`super::executor::BrainExportExecutor`] on a
//! worker). Local mode: [`LocalBrainScheduler`] runs the export inline as a
//! tokio task. Both consider a brain due when `export_interval_secs > 0`
//! and no export started within the interval; the per-brain lock plus the
//! exporter's no-op detection make an accidental double export harmless.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tracing::{info, warn};

use pensieve_brain::registry::BrainRecord;

use super::BrainState;

fn interval_due(interval_secs: u64, last: Option<&str>) -> bool {
    if interval_secs == 0 {
        return false;
    }
    let Some(last) = last.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) else {
        return true;
    };
    let elapsed = Utc::now().signed_duration_since(last.with_timezone(&Utc));
    elapsed.num_seconds() >= interval_secs as i64
}

fn due(rec: &BrainRecord) -> bool {
    interval_due(rec.config.export_interval_secs, rec.runtime.last_export_at.as_deref())
}

fn gardener_due(rec: &BrainRecord) -> bool {
    rec.config.gardener.enabled
        && interval_due(rec.config.gardener.interval_secs, rec.runtime.last_gardener_at.as_deref())
}

/// Hosted-mode scheduler: enqueues fabric jobs on an interval.
pub struct BrainScheduler {
    state: BrainState,
    fabric: Arc<pensieve_catalog::PgFabricStore>,
    pub poll: Duration,
}

impl BrainScheduler {
    pub fn new(state: BrainState, fabric: Arc<pensieve_catalog::PgFabricStore>) -> Self {
        Self { state, fabric, poll: Duration::from_secs(60) }
    }

    pub async fn tick_once(&self) -> anyhow::Result<()> {
        if self.state.git.is_none() {
            return Ok(());
        }
        for rec in self.state.registry.list().await? {
            if gardener_due(&rec) {
                match super::routes::trigger_gardener(&self.state, &rec.config).await {
                    Ok(v) => info!(brain = %rec.config.name, result = %v, "gardener run scheduled"),
                    Err(e) => warn!(brain = %rec.config.name, error = %e, "gardener trigger failed"),
                }
            }
            if !due(&rec) {
                continue;
            }
            let enqueued = self
                .fabric
                .enqueue_job(
                    self.state.agent.tenant,
                    &pensieve_core::fabric::EnqueueJob {
                        kind: pensieve_core::fabric::JOB_BRAIN_EXPORT.to_string(),
                        payload: json!({ "brain": rec.config.name }),
                        priority: 0,
                        affinity_worker_id: None,
                        req_capabilities: vec!["brain".into()],
                        label_selector: json!({}),
                        max_attempts: 1,
                    },
                )
                .await?;
            if let Some(job_id) = enqueued {
                info!(brain = %rec.config.name, job_id = %job_id, "brain export scheduled");
            }
        }
        Ok(())
    }

    pub async fn run(self, shutdown: impl std::future::Future<Output = ()>) {
        info!("brain scheduler starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("brain scheduler shutdown"); return; }
                _ = tokio::time::sleep(self.poll) => {
                    if let Err(e) = self.tick_once().await {
                        warn!(error = %e, "brain scheduler tick failed");
                    }
                }
            }
        }
    }
}

/// Local-mode scheduler: runs due exports inline (one task per due brain;
/// the per-brain lock serializes against manual triggers and pushes).
pub struct LocalBrainScheduler {
    state: BrainState,
    pub poll: Duration,
}

impl LocalBrainScheduler {
    pub fn new(state: BrainState) -> Self {
        Self { state, poll: Duration::from_secs(60) }
    }

    pub async fn tick_once(&self) {
        if self.state.git.is_none() {
            return;
        }
        let brains = match self.state.registry.list().await {
            Ok(b) => b,
            Err(e) => {
                warn!(error = %e, "brain scheduler: registry list failed");
                return;
            }
        };
        for rec in brains {
            if gardener_due(&rec) {
                match super::routes::trigger_gardener(&self.state, &rec.config).await {
                    Ok(v) => info!(brain = %rec.config.name, result = %v, "gardener run scheduled"),
                    Err(e) => warn!(brain = %rec.config.name, error = %e, "gardener trigger failed"),
                }
            }
            if !due(&rec) {
                continue;
            }
            let state = self.state.clone();
            tokio::spawn(async move {
                match super::routes::run_export_now(&state, &rec.config).await {
                    Ok(out) => info!(brain = %rec.config.name, result = %out, "scheduled brain export finished"),
                    Err(e) => warn!(brain = %rec.config.name, error = %e, "scheduled brain export failed"),
                }
            });
        }
    }

    pub async fn run(self, shutdown: impl std::future::Future<Output = ()>) {
        info!("local brain scheduler starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("local brain scheduler shutdown"); return; }
                _ = tokio::time::sleep(self.poll) => self.tick_once().await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::due;
    use pensieve_brain::registry::{BrainConfig, BrainRecord, BrainRuntime, RealmSelector};

    fn rec(interval: u64, last: Option<&str>) -> BrainRecord {
        let mut config =
            BrainConfig::new("t", RealmSelector::All, "2026-07-08T00:00:00Z").unwrap();
        config.export_interval_secs = interval;
        let mut runtime = BrainRuntime::default();
        runtime.last_export_at = last.map(str::to_string);
        BrainRecord { config, runtime }
    }

    #[test]
    fn due_logic() {
        assert!(!due(&rec(0, None)), "manual-only brains never scheduled");
        assert!(due(&rec(900, None)), "never exported ⇒ due");
        assert!(due(&rec(900, Some("2000-01-01T00:00:00Z"))), "stale ⇒ due");
        let just_now = chrono::Utc::now().to_rfc3339();
        assert!(!due(&rec(900, Some(&just_now))), "fresh ⇒ not due");
    }
}
