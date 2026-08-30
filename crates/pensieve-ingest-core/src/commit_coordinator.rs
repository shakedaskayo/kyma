//! Shared commit coordinator — bundles extents from concurrent flushes
//! into a single catalog snapshot per table.
//!
//! # Why
//!
//! The staging buffer already bundles *ingest requests* into one *extent*.
//! The remaining bottleneck is the per-table `tables.current_snapshot_id`
//! CAS: every flush that commits its own extent competes for the same
//! row, and concurrent flushes serialize.
//!
//! This coordinator moves to "group-commit squared": flushes hand their
//! extent manifests to a central channel; a background task drains the
//! channel every ~5ms, groups pending jobs by table_id, and commits each
//! group as **one snapshot containing N extents**. All flushes whose
//! extents landed in the same commit share one `SnapshotId` and one CAS.
//!
//! # Ordering + consistency
//!
//! Extents within a snapshot are unordered (they're all committed
//! atomically). Across snapshots, the commit coordinator processes jobs
//! roughly in arrival order — good enough for ingest where "arrival order"
//! is already a loose concept across concurrent clients.
//!
//! On CAS conflict (another writer committed elsewhere), we retry the
//! entire group with exponential backoff + jitter, same as the per-flush
//! path did before.
//!
//! # Failure model
//!
//! If the coordinator task panics, the channel is closed and submitters
//! observe a send-error → they fall back to committing directly through
//! `catalog.begin_snapshot`. This preserves correctness but loses the
//! grouping benefit until the coordinator restarts.

use pensieve_core::catalog::{Catalog, ExtentManifest, SnapshotSummary};
use pensieve_core::errors::{CatalogError, Error, Result};
use pensieve_core::types::{SnapshotId, TableId};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, instrument, warn};

/// A single extent waiting to be committed.
pub struct CommitJob {
    pub table_id: TableId,
    pub manifest: ExtentManifest,
    pub rows: u64,
    pub bytes: u64,
    /// Tag so observability + metric labels stay informative.
    pub table_label: String,
    /// Where to send the resulting snapshot_id back to the waiter.
    pub responder: oneshot::Sender<Result<SnapshotId>>,
}

/// Config for the coordinator's batching behavior.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// Max wall-clock to wait after the first job arrives before
    /// committing the current batch. Lower = less latency, less
    /// amortization. Higher = more throughput, more latency.
    pub batch_window: Duration,
    /// Hard cap on number of extents per snapshot — prevents one
    /// slow coordinator cycle from pathologically large commits.
    pub max_extents_per_snapshot: usize,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            batch_window: Duration::from_millis(5),
            max_extents_per_snapshot: 128,
        }
    }
}

impl CoordinatorConfig {
    pub fn from_env() -> Self {
        let mut d = Self::default();
        if let Ok(v) = std::env::var("PENSIEVE_COMMIT_WINDOW_MS")
            .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
        {
            d.batch_window = Duration::from_millis(v);
        }
        if let Ok(v) = std::env::var("PENSIEVE_COMMIT_MAX_EXTENTS").and_then(|v| {
            v.parse::<usize>()
                .map_err(|_| std::env::VarError::NotPresent)
        }) {
            d.max_extents_per_snapshot = v;
        }
        d
    }
}

/// Handle to a running coordinator. Cheap to clone.
#[derive(Clone)]
pub struct CommitCoordinator {
    tx: mpsc::UnboundedSender<CommitJob>,
}

impl CommitCoordinator {
    /// Spawn a coordinator task. It runs until the returned handle (and
    /// all its clones) are dropped.
    pub fn spawn(catalog: Arc<dyn Catalog>, config: CoordinatorConfig) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<CommitJob>();
        tokio::spawn(run_coordinator(catalog, rx, config));
        Self { tx }
    }

    /// Submit an extent for commit. Resolves with the committed snapshot_id
    /// (shared with every other job that landed in the same batch/table).
    pub async fn submit(
        &self,
        job_fn: impl FnOnce(oneshot::Sender<Result<SnapshotId>>) -> CommitJob,
    ) -> Result<SnapshotId> {
        let (tx, rx) = oneshot::channel();
        let job = job_fn(tx);
        self.tx
            .send(job)
            .map_err(|_| Error::Internal("commit coordinator channel closed".into()))?;
        match rx.await {
            Ok(res) => res,
            Err(_) => Err(Error::Internal(
                "commit coordinator dropped responder".into(),
            )),
        }
    }
}

#[instrument(skip(catalog, rx, config))]
async fn run_coordinator(
    catalog: Arc<dyn Catalog>,
    mut rx: mpsc::UnboundedReceiver<CommitJob>,
    config: CoordinatorConfig,
) {
    info!(
        batch_window_ms = config.batch_window.as_millis() as u64,
        max_extents = config.max_extents_per_snapshot,
        "commit coordinator starting"
    );
    loop {
        let first = match rx.recv().await {
            Some(j) => j,
            None => {
                info!("commit coordinator channel closed; exiting");
                return;
            }
        };
        let deadline = Instant::now() + config.batch_window;
        let mut batch = vec![first];

        // Drain as many additional jobs as arrive within the window, up to
        // the hard cap. This is the "wait a little, then take what's there"
        // pattern — same shape as Postgres's `commit_delay`.
        while batch.len() < config.max_extents_per_snapshot {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, rx.recv()).await {
                Ok(Some(job)) => batch.push(job),
                Ok(None) => break, // channel closed
                Err(_) => break,   // timed out
            }
        }

        process_batch(&*catalog, batch).await;
    }
}

async fn process_batch(catalog: &dyn Catalog, batch: Vec<CommitJob>) {
    // Group by table. Each table gets one CAS snapshot containing all its
    // queued extents.
    let mut by_table: HashMap<TableId, Vec<CommitJob>> = HashMap::new();
    for job in batch {
        by_table.entry(job.table_id).or_default().push(job);
    }

    // Different tables don't contend on CAS. We could spawn, but `catalog`
    // here is `&dyn Catalog`, not `'static`. Sequential per batch is fine —
    // the NEXT batch begins as soon as this one returns, and multi-table
    // workloads aren't our current bottleneck anyway.
    for (table_id, jobs) in by_table {
        commit_one_table_group(catalog, table_id, jobs).await;
    }
}

async fn commit_one_table_group(catalog: &dyn Catalog, table_id: TableId, jobs: Vec<CommitJob>) {
    let extent_count = jobs.len();
    let total_rows: i64 = jobs.iter().map(|j| j.rows as i64).sum();
    let total_bytes: i64 = jobs.iter().map(|j| j.bytes as i64).sum();
    let label = jobs
        .first()
        .map(|j| j.table_label.clone())
        .unwrap_or_default();

    debug!(
        table_id = %table_id,
        extent_count,
        total_rows,
        "coordinator committing grouped snapshot"
    );

    const MAX_ATTEMPTS: u32 = 20;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut txn = match catalog.begin_snapshot(table_id).await {
            Ok(t) => t,
            Err(e) => {
                warn!(error = %e, "coordinator begin_snapshot failed");
                fail_all(jobs, Error::Catalog(CatalogError::Sql(format!("{e}"))));
                return;
            }
        };
        let mut add_err: Option<Error> = None;
        for job in &jobs {
            if let Err(e) = txn.add_extent(job.manifest.clone()).await {
                add_err = Some(e);
                break;
            }
        }
        if let Some(e) = add_err {
            warn!(error = %e, "coordinator add_extent failed; rolling back");
            let _ = txn.rollback().await;
            fail_all(jobs, e);
            return;
        }

        match txn
            .commit(SnapshotSummary {
                rows_added: total_rows,
                bytes_added: total_bytes,
                operation: format!("ingest_x{extent_count}"),
                ..Default::default()
            })
            .await
        {
            Ok(snapshot_id) => {
                ::metrics::counter!(
                    "pensieve_commit_batches_total",
                    "table" => label.clone()
                )
                .increment(1);
                ::metrics::histogram!(
                    "pensieve_commit_batch_extents",
                    "table" => label.clone()
                )
                .record(extent_count as f64);
                for job in jobs {
                    let _ = job.responder.send(Ok(snapshot_id));
                }
                return;
            }
            Err(Error::Catalog(CatalogError::Conflict)) => {
                ::metrics::counter!(
                    "pensieve_catalog_cas_conflicts_total",
                    "table" => label.clone()
                )
                .increment(1);
                if attempt >= MAX_ATTEMPTS {
                    warn!("commit coordinator exhausted retries; failing batch");
                    fail_all(jobs, Error::Catalog(CatalogError::Conflict));
                    return;
                }
                let base_ms = (2u64.saturating_pow(attempt.min(8))).min(200);
                let jitter_ms = fastrand::u64(..base_ms.max(1));
                tokio::time::sleep(Duration::from_millis(base_ms + jitter_ms)).await;
                continue;
            }
            Err(e) => {
                warn!(error = %e, "coordinator commit failed");
                fail_all(jobs, e);
                return;
            }
        }
    }
}

fn fail_all(jobs: Vec<CommitJob>, err: Error) {
    // `Error` isn't `Clone`. Send the first responder the real error; the
    // rest receive a generic "batch failed" variant. Callers fall back to
    // retrying on their own.
    let msg = format!("commit batch failed: {err}");
    let mut iter = jobs.into_iter();
    if let Some(first) = iter.next() {
        let _ = first.responder.send(Err(err));
    }
    for rest in iter {
        let _ = rest.responder.send(Err(Error::Internal(msg.clone())));
    }
}
