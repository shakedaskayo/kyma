//! Generic internal queueing + worker coordination.
//!
//! A [`Queue`] accepts jobs (`partition` + JSON payload), groups them into
//! batches (size threshold OR linger window, whichever first), and hands each
//! batch to a [`BatchHandler`] on a background worker. Callers get their
//! sequence number back immediately — the write is asynchronous by default.
//!
//! Two durability tiers, chosen per job at submit time:
//!
//! - **Best-effort** (`durable = false`): the job lives only in the in-process
//!   channel. A hard crash inside the linger window loses it. Right for
//!   regenerable work (e.g. consolidator-extracted memories).
//! - **Durable** (`durable = true`, requires a catalog): the job is persisted
//!   to the catalog's existing `background_tasks` queue *before* submit
//!   returns. A crash replays it on the next worker start (lease-expiry +
//!   attempt tracking + terminal `failed` status — i.e. recovery, retry, and
//!   DLQ — come from the catalog implementation, both SQLite and Postgres).
//!
//! Ordering: jobs are drained in submit (sequence) order by a single worker,
//! so FIFO holds globally and therefore per-partition. Replays from the
//! durable store are claimed `ORDER BY priority DESC, created_at ASC`, which
//! preserves submit order for same-priority jobs.
//!
//! Read-your-own-writes: [`Queue::barrier`] waits (bounded) until every job
//! submitted to the named partitions before the call has been *attempted*
//! (committed, or parked for durable retry / dropped after best-effort
//! retries). It is a no-op when nothing is pending — the common case.

mod worker;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kyma_core::catalog::Catalog;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Instant;
use uuid::Uuid;

pub use worker::spawn;

/// Errors surfaced to submitters. Callers typically fall back to their
/// synchronous path on any of these.
#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    /// The in-process channel is full and the job has no durable backing.
    #[error("queue '{0}' is full")]
    Full(String),
    /// The worker has stopped (shutdown raced the submit).
    #[error("queue '{0}' worker has stopped")]
    Closed(String),
    /// The durable store rejected the enqueue.
    #[error("durable queue store: {0}")]
    Store(String),
}

/// Flush/batch tuning for one queue.
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Queue name — doubles as the `background_tasks.kind` for durable jobs
    /// and as the metric label.
    pub name: String,
    /// Max jobs per batch handed to the handler.
    pub max_batch: usize,
    /// How long the worker lingers after the first job of a batch to let
    /// concurrent submitters coalesce.
    pub linger: Duration,
    /// In-process channel capacity. Full ⇒ durable jobs ride the repair poll,
    /// best-effort jobs get [`QueueError::Full`].
    pub channel_cap: usize,
    /// In-process retries per batch before durable jobs are parked back to
    /// the store and best-effort jobs are dropped.
    pub max_retries: u32,
    /// Upper bound a [`Queue::barrier`] waits before giving up.
    pub barrier_timeout: Duration,
    /// How often the worker polls the durable store for replayed / requeued /
    /// overflow jobs. Irrelevant without a durable store.
    pub poll_interval: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            name: "queue".to_string(),
            max_batch: 64,
            linger: Duration::from_millis(50),
            channel_cap: 1024,
            max_retries: 3,
            barrier_timeout: Duration::from_secs(2),
            poll_interval: Duration::from_secs(2),
        }
    }
}

impl QueueConfig {
    /// Build a named config, overriding defaults from `<PREFIX>_MAX_BATCH`,
    /// `<PREFIX>_LINGER_MS`, `<PREFIX>_CAP`, `<PREFIX>_MAX_RETRIES`,
    /// `<PREFIX>_BARRIER_MS`, `<PREFIX>_POLL_MS` when set.
    pub fn from_env(name: &str, prefix: &str) -> Self {
        fn num(prefix: &str, key: &str) -> Option<u64> {
            std::env::var(format!("{prefix}_{key}")).ok()?.parse().ok()
        }
        let mut c = Self {
            name: name.to_string(),
            ..Self::default()
        };
        if let Some(v) = num(prefix, "MAX_BATCH") {
            c.max_batch = (v as usize).max(1);
        }
        if let Some(v) = num(prefix, "LINGER_MS") {
            c.linger = Duration::from_millis(v);
        }
        if let Some(v) = num(prefix, "CAP") {
            c.channel_cap = (v as usize).max(1);
        }
        if let Some(v) = num(prefix, "MAX_RETRIES") {
            c.max_retries = v as u32;
        }
        if let Some(v) = num(prefix, "BARRIER_MS") {
            c.barrier_timeout = Duration::from_millis(v);
        }
        if let Some(v) = num(prefix, "POLL_MS") {
            c.poll_interval = Duration::from_millis(v);
        }
        c
    }
}

/// One queued unit of work as seen by the [`BatchHandler`].
#[derive(Debug, Clone)]
pub struct Job {
    /// Monotonic submit order within this process. Replayed durable jobs get
    /// a fresh seq (their original submitter is gone).
    pub seq: u64,
    /// Ordering/consistency scope (e.g. a memory realm).
    pub partition: String,
    pub payload: Value,
    pub durable: bool,
    /// Backing `background_tasks` row for durable jobs.
    pub task_id: Option<Uuid>,
}

/// Processes one drained batch, in order. An `Err` fails the whole batch:
/// the worker retries in-process up to `max_retries`, then parks durable jobs
/// back to the store (catalog requeue → eventual `failed`/DLQ) and drops
/// best-effort jobs with an error log.
#[async_trait::async_trait]
pub trait BatchHandler: Send + Sync + 'static {
    async fn handle(&self, jobs: &[Job]) -> anyhow::Result<()>;
}

/// Per-partition high-water marks driving [`Queue::barrier`].
#[derive(Default)]
struct Watermarks {
    /// Highest seq successfully placed on the channel, per partition.
    submitted: HashMap<String, u64>,
    /// Highest seq the worker has attempted (committed or parked), per
    /// partition.
    attempted: HashMap<String, u64>,
}

struct Shared {
    cfg: QueueConfig,
    seq: AtomicU64,
    marks: Mutex<Watermarks>,
    notify: tokio::sync::Notify,
    durable: Option<Arc<dyn Catalog>>,
}

/// Cheap-to-clone handle for submitting jobs and awaiting flush barriers.
#[derive(Clone)]
pub struct Queue {
    shared: Arc<Shared>,
    tx: mpsc::Sender<Job>,
}

impl Queue {
    /// Enqueue a job and return its sequence number immediately.
    ///
    /// `durable = true` persists the job to the catalog's background task
    /// queue *before* returning, so an ack means the job survives a crash.
    /// Without a durable store configured, durable degrades to best-effort.
    pub async fn submit(
        &self,
        partition: &str,
        payload: Value,
        durable: bool,
    ) -> Result<u64, QueueError> {
        let seq = self.shared.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let task_id = if durable {
            match &self.shared.durable {
                Some(catalog) => {
                    let envelope = json!({"partition": partition, "payload": payload});
                    let id = catalog
                        .submit_task(&self.shared.cfg.name, None, envelope, 0)
                        .await
                        .map_err(|e| QueueError::Store(e.to_string()))?;
                    Some(id)
                }
                None => None,
            }
        } else {
            None
        };
        let job = Job {
            seq,
            partition: partition.to_string(),
            payload,
            durable,
            task_id,
        };
        match self.tx.try_send(job) {
            Ok(()) => {
                let mut marks = self.shared.marks.lock().expect("queue marks lock");
                let e = marks.submitted.entry(partition.to_string()).or_insert(0);
                if seq > *e {
                    *e = seq;
                }
                ::metrics::counter!(
                    "kyma_queue_jobs_submitted_total",
                    "queue" => self.shared.cfg.name.clone()
                )
                .increment(1);
                Ok(seq)
            }
            Err(mpsc::error::TrySendError::Full(_)) if task_id.is_some() => {
                // Durable row exists; the worker's repair poll will claim it.
                // No submitted watermark: under overload the barrier does not
                // cover this job (bounded inconsistency, documented).
                ::metrics::counter!(
                    "kyma_queue_overflow_to_store_total",
                    "queue" => self.shared.cfg.name.clone()
                )
                .increment(1);
                Ok(seq)
            }
            Err(mpsc::error::TrySendError::Full(_)) => {
                Err(QueueError::Full(self.shared.cfg.name.clone()))
            }
            Err(mpsc::error::TrySendError::Closed(job)) => {
                // Worker is gone; if we persisted a row, retire it so a later
                // process doesn't replay work the caller will redo sync.
                if let (Some(catalog), Some(tid)) = (&self.shared.durable, job.task_id) {
                    let _ = catalog.complete_task(tid).await;
                }
                Err(QueueError::Closed(self.shared.cfg.name.clone()))
            }
        }
    }

    /// Wait (bounded by `barrier_timeout`) until every job submitted to the
    /// given partitions before this call has been attempted. An empty slice
    /// means "all partitions". Returns `false` on timeout.
    pub async fn barrier(&self, partitions: &[String]) -> bool {
        self.barrier_with(partitions, self.shared.cfg.barrier_timeout)
            .await
    }

    /// [`Queue::barrier`] with an explicit timeout.
    pub async fn barrier_with(&self, partitions: &[String], timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let targets: Vec<(String, u64)> = {
            let marks = self.shared.marks.lock().expect("queue marks lock");
            if partitions.is_empty() {
                marks
                    .submitted
                    .iter()
                    .map(|(p, s)| (p.clone(), *s))
                    .collect()
            } else {
                partitions
                    .iter()
                    .filter_map(|p| marks.submitted.get(p).map(|s| (p.clone(), *s)))
                    .collect()
            }
        };
        if targets.is_empty() {
            return true;
        }
        let reached = |marks: &Watermarks| {
            targets
                .iter()
                .all(|(p, t)| marks.attempted.get(p).copied().unwrap_or(0) >= *t)
        };
        let start = Instant::now();
        loop {
            let notified = self.shared.notify.notified();
            tokio::pin!(notified);
            // Register interest *before* checking, so a notify between the
            // check and the await is not lost.
            notified.as_mut().enable();
            {
                let marks = self.shared.marks.lock().expect("queue marks lock");
                if reached(&marks) {
                    ::metrics::histogram!(
                        "kyma_queue_barrier_wait_seconds",
                        "queue" => self.shared.cfg.name.clone()
                    )
                    .record(start.elapsed().as_secs_f64());
                    return true;
                }
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                ::metrics::counter!(
                    "kyma_queue_barrier_timeouts_total",
                    "queue" => self.shared.cfg.name.clone()
                )
                .increment(1);
                return false;
            };
            if tokio::time::timeout(remaining, notified).await.is_err() {
                let marks = self.shared.marks.lock().expect("queue marks lock");
                let ok = reached(&marks);
                if !ok {
                    ::metrics::counter!(
                        "kyma_queue_barrier_timeouts_total",
                        "queue" => self.shared.cfg.name.clone()
                    )
                    .increment(1);
                }
                return ok;
            }
        }
    }

    /// Drain every partition — used at shutdown / stdio EOF. Returns `false`
    /// if pending work could not be flushed within `timeout`.
    pub async fn drain(&self, timeout: Duration) -> bool {
        self.barrier_with(&[], timeout).await
    }

    /// The queue's configured name.
    pub fn name(&self) -> &str {
        &self.shared.cfg.name
    }
}

#[cfg(test)]
mod tests;
