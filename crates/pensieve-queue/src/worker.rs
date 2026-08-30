//! The queue's single background worker: drains submit-ordered batches,
//! invokes the handler with in-process retries, settles durable rows, and
//! advances the per-partition watermarks that back [`Queue::barrier`].

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use kyma_core::catalog::BackgroundTask;
use kyma_core::types::NodeId;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, error, info, warn};

use crate::{BatchHandler, Job, Queue, QueueConfig, Shared, Watermarks};

/// Build a queue and spawn its worker. The worker runs until `shutdown`
/// resolves (draining whatever is still queued) or every [`Queue`] handle is
/// dropped.
///
/// With a `durable` catalog the worker first replays jobs left behind by a
/// previous process, then keeps a slow repair poll going for requeued /
/// overflow jobs. Without one, durable submits silently degrade to
/// best-effort.
pub fn spawn(
    cfg: QueueConfig,
    durable: Option<Arc<dyn kyma_core::catalog::Catalog>>,
    handler: Arc<dyn BatchHandler>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> (Queue, JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(cfg.channel_cap);
    let shared = Arc::new(Shared {
        cfg,
        seq: std::sync::atomic::AtomicU64::new(0),
        marks: std::sync::Mutex::new(Watermarks::default()),
        notify: tokio::sync::Notify::new(),
        durable,
    });
    let queue = Queue {
        shared: shared.clone(),
        tx,
    };
    let handle = tokio::spawn(run(shared, rx, handler, shutdown));
    (queue, handle)
}

async fn run(
    shared: Arc<Shared>,
    mut rx: mpsc::Receiver<Job>,
    handler: Arc<dyn BatchHandler>,
    shutdown: impl Future<Output = ()> + Send,
) {
    let node_id = NodeId::new();
    info!(
        queue = %shared.cfg.name,
        durable = shared.durable.is_some(),
        max_batch = shared.cfg.max_batch,
        linger_ms = shared.cfg.linger.as_millis() as u64,
        "queue worker starting"
    );
    // Replay whatever a previous process left behind before live traffic.
    if shared.durable.is_some() {
        claim_from_store(&shared, &handler, node_id).await;
    }

    tokio::pin!(shutdown);
    let mut poll = tokio::time::interval(shared.cfg.poll_interval);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => {
                debug!(queue = %shared.cfg.name, "queue worker shutting down; draining");
                drain_channel(&shared, &mut rx, &handler).await;
                return;
            }
            maybe = rx.recv() => {
                let Some(first) = maybe else {
                    // Every handle dropped — nothing more can arrive.
                    return;
                };
                let batch = collect_batch(&mut rx, first, &shared.cfg).await;
                process_batch(&shared, &handler, batch).await;
            }
            _ = poll.tick(), if shared.durable.is_some() => {
                // Repair pass: requeued failures, channel-overflow spills, and
                // lease-expired claims from dead peers. Skipped while live
                // traffic is waiting so claims can't race the channel path.
                if rx.is_empty() {
                    claim_from_store(&shared, &handler, node_id).await;
                }
            }
        }
    }
}

/// Greedily extend a batch until `max_batch` or the linger deadline.
async fn collect_batch(rx: &mut mpsc::Receiver<Job>, first: Job, cfg: &QueueConfig) -> Vec<Job> {
    let mut batch = vec![first];
    let deadline = Instant::now() + cfg.linger;
    while batch.len() < cfg.max_batch {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(job)) => batch.push(job),
            Ok(None) | Err(_) => break,
        }
    }
    batch
}

/// Drain everything already queued (no linger — we're exiting) in
/// `max_batch` chunks.
async fn drain_channel(
    shared: &Arc<Shared>,
    rx: &mut mpsc::Receiver<Job>,
    handler: &Arc<dyn BatchHandler>,
) {
    loop {
        let mut batch = Vec::new();
        while batch.len() < shared.cfg.max_batch {
            match rx.try_recv() {
                Ok(job) => batch.push(job),
                Err(_) => break,
            }
        }
        if batch.is_empty() {
            return;
        }
        process_batch(shared, handler, batch).await;
    }
}

/// Run one batch through the handler with bounded in-process retries, then
/// settle durable rows and advance watermarks. Failed durable jobs are parked
/// back to the store (requeue → eventual `failed`/DLQ); failed best-effort
/// jobs are dropped with an error log. Watermarks advance either way —
/// `barrier` means "attempted", not "guaranteed committed".
async fn process_batch(shared: &Arc<Shared>, handler: &Arc<dyn BatchHandler>, batch: Vec<Job>) {
    let started = Instant::now();
    let mut attempt: u32 = 0;
    let ok = loop {
        match handler.handle(&batch).await {
            Ok(()) => break true,
            Err(e) => {
                attempt += 1;
                if attempt > shared.cfg.max_retries {
                    error!(
                        queue = %shared.cfg.name,
                        jobs = batch.len(),
                        error = %e,
                        "batch failed after retries; parking durable jobs, dropping best-effort jobs"
                    );
                    break false;
                }
                let base_ms = 50u64.saturating_mul(2u64.saturating_pow(attempt.min(6)));
                let jitter = fastrand::u64(..base_ms.max(1));
                warn!(
                    queue = %shared.cfg.name,
                    attempt,
                    error = %e,
                    "batch failed; retrying"
                );
                ::metrics::counter!(
                    "kyma_queue_batch_retries_total",
                    "queue" => shared.cfg.name.clone()
                )
                .increment(1);
                tokio::time::sleep(Duration::from_millis(base_ms + jitter)).await;
            }
        }
    };

    if let Some(catalog) = &shared.durable {
        for job in &batch {
            let Some(tid) = job.task_id else { continue };
            let res = if ok {
                catalog.complete_task(tid).await
            } else {
                catalog
                    .fail_task(tid, "batch failed after in-process retries")
                    .await
            };
            if let Err(e) = res {
                warn!(queue = %shared.cfg.name, task_id = %tid, error = %e, "settling durable job failed");
            }
        }
    }
    if !ok {
        ::metrics::counter!(
            "kyma_queue_batches_failed_total",
            "queue" => shared.cfg.name.clone()
        )
        .increment(1);
    }

    {
        let mut marks = shared.marks.lock().expect("queue marks lock");
        for job in &batch {
            let e = marks.attempted.entry(job.partition.clone()).or_insert(0);
            if job.seq > *e {
                *e = job.seq;
            }
        }
    }
    shared.notify.notify_waiters();

    ::metrics::counter!(
        "kyma_queue_batches_total",
        "queue" => shared.cfg.name.clone()
    )
    .increment(1);
    ::metrics::histogram!(
        "kyma_queue_batch_jobs",
        "queue" => shared.cfg.name.clone()
    )
    .record(batch.len() as f64);
    ::metrics::histogram!(
        "kyma_queue_batch_duration_seconds",
        "queue" => shared.cfg.name.clone()
    )
    .record(started.elapsed().as_secs_f64());
}

/// Claim and process store-backed jobs until the store runs dry. Each claim
/// bumps the row's attempt count; the catalog parks rows as `failed` once
/// `max_attempts` is exhausted, so this always terminates.
async fn claim_from_store(shared: &Arc<Shared>, handler: &Arc<dyn BatchHandler>, node_id: NodeId) {
    let Some(catalog) = shared.durable.clone() else {
        return;
    };
    // Lease long enough to ride out a slow batch (cold embedding model etc.).
    let lease = chrono::Duration::seconds(120);
    loop {
        let mut batch: Vec<Job> = Vec::new();
        while batch.len() < shared.cfg.max_batch {
            match catalog.claim_task(&shared.cfg.name, node_id, lease).await {
                Ok(Some(task)) => batch.push(job_from_task(shared, task)),
                Ok(None) => break,
                Err(e) => {
                    warn!(queue = %shared.cfg.name, error = %e, "claiming durable jobs failed");
                    break;
                }
            }
        }
        if batch.is_empty() {
            return;
        }
        debug!(queue = %shared.cfg.name, jobs = batch.len(), "replaying durable jobs from store");
        ::metrics::counter!(
            "kyma_queue_jobs_replayed_total",
            "queue" => shared.cfg.name.clone()
        )
        .increment(batch.len() as u64);
        process_batch(shared, handler, batch).await;
    }
}

/// Rehydrate a store row into a [`Job`]. The original submitter (and its
/// barrier waiters) are gone, so the fresh seq is deliberately not recorded
/// as "submitted" — it only advances the attempted watermark.
fn job_from_task(shared: &Arc<Shared>, task: BackgroundTask) -> Job {
    let seq = shared.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let (partition, payload) = match &task.payload {
        Value::Object(map) => (
            map.get("partition")
                .and_then(Value::as_str)
                .unwrap_or("default")
                .to_string(),
            map.get("payload").cloned().unwrap_or(Value::Null),
        ),
        other => ("default".to_string(), other.clone()),
    };
    Job {
        seq,
        partition,
        payload,
        durable: true,
        task_id: Some(task.id),
    }
}
