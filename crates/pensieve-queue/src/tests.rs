//! In-memory + sqlite-durable coverage for the queue: FIFO order, coalescing,
//! barrier semantics, retry/drop, shutdown drain, and store replay.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;

use crate::{spawn, BatchHandler, Job, Queue, QueueConfig};

struct TestHandler {
    calls: AtomicU32,
    fail_first: u32,
    jobs_seen: Mutex<Vec<Job>>,
    batch_sizes: Mutex<Vec<usize>>,
    gate: Mutex<Option<Arc<tokio::sync::Semaphore>>>,
}

impl TestHandler {
    fn new() -> Arc<Self> {
        Self::failing(0)
    }

    fn failing(fail_first: u32) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicU32::new(0),
            fail_first,
            jobs_seen: Mutex::new(Vec::new()),
            batch_sizes: Mutex::new(Vec::new()),
            gate: Mutex::new(None),
        })
    }

    fn gated() -> (Arc<Self>, Arc<tokio::sync::Semaphore>) {
        let h = Self::new();
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        *h.gate.lock().unwrap() = Some(gate.clone());
        (h, gate)
    }

    fn seen_payloads(&self) -> Vec<serde_json::Value> {
        self.jobs_seen
            .lock()
            .unwrap()
            .iter()
            .map(|j| j.payload.clone())
            .collect()
    }
}

#[async_trait::async_trait]
impl BatchHandler for TestHandler {
    async fn handle(&self, jobs: &[Job]) -> anyhow::Result<()> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.acquire().await.expect("gate closed").forget();
        }
        if call <= self.fail_first {
            anyhow::bail!("induced failure #{call}");
        }
        self.batch_sizes.lock().unwrap().push(jobs.len());
        self.jobs_seen.lock().unwrap().extend(jobs.iter().cloned());
        Ok(())
    }
}

fn test_cfg(name: &str) -> QueueConfig {
    QueueConfig {
        name: name.to_string(),
        max_batch: 64,
        linger: Duration::from_millis(20),
        channel_cap: 256,
        max_retries: 2,
        barrier_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(50),
    }
}

fn spawn_in_memory(name: &str, handler: Arc<TestHandler>) -> (Queue, tokio::task::JoinHandle<()>) {
    let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    // Leak the sender so the shutdown future stays pending for the test's life.
    std::mem::forget(_shutdown_tx);
    spawn(test_cfg(name), None, handler, async move {
        let _ = shutdown_rx.await;
    })
}

/// Poll until `cond` or `timeout`. Returns whether the condition was reached.
async fn eventually(timeout: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cond()
}

#[tokio::test]
async fn fifo_order_preserved_and_batched() {
    let handler = TestHandler::new();
    let (queue, _h) = spawn_in_memory("t_fifo", handler.clone());

    for i in 0..10 {
        queue.submit("a", json!(i), false).await.expect("submit");
    }
    assert!(queue.barrier(&["a".into()]).await, "barrier should flush");

    let payloads = handler.seen_payloads();
    assert_eq!(payloads, (0..10).map(|i| json!(i)).collect::<Vec<_>>());
    // Back-to-back submits within one linger window coalesce into few batches.
    let batches = handler.batch_sizes.lock().unwrap().len();
    assert!(batches <= 3, "expected coalesced batches, got {batches}");
}

#[tokio::test]
async fn barrier_is_noop_when_idle_and_scoped_by_partition() {
    let (handler, gate) = TestHandler::gated();
    let (queue, _h) = spawn_in_memory("t_scope", handler.clone());

    // Nothing pending anywhere: instant true.
    assert!(queue.barrier(&[]).await);

    queue
        .submit("stuck", json!("x"), false)
        .await
        .expect("submit");
    // Partition with no submissions is never blocked by another's backlog.
    assert!(queue.barrier(&["other".into()]).await);
    // The stuck partition times out while the handler is gated.
    assert!(
        !queue
            .barrier_with(&["stuck".into()], Duration::from_millis(100))
            .await
    );
    gate.add_permits(1);
    assert!(queue.barrier(&["stuck".into()]).await);
}

#[tokio::test]
async fn failed_batch_drops_after_retries_then_recovers() {
    // 3 attempts (initial + max_retries=2) all fail → batch dropped.
    let handler = TestHandler::failing(3);
    let (queue, _h) = spawn_in_memory("t_retry", handler.clone());

    queue
        .submit("a", json!("doomed"), false)
        .await
        .expect("submit");
    assert!(
        queue.barrier(&["a".into()]).await,
        "barrier covers attempted (parked) jobs"
    );
    assert_eq!(handler.calls.load(Ordering::SeqCst), 3);
    assert!(handler.seen_payloads().is_empty());

    // The worker keeps serving after a dropped batch.
    queue.submit("a", json!("ok"), false).await.expect("submit");
    assert!(queue.barrier(&["a".into()]).await);
    assert_eq!(handler.seen_payloads(), vec![json!("ok")]);
}

#[tokio::test]
async fn shutdown_drains_pending_jobs() {
    let handler = TestHandler::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let mut cfg = test_cfg("t_drain");
    cfg.linger = Duration::from_millis(500);
    let (queue, handle) = spawn(cfg, None, handler.clone(), async move {
        let _ = shutdown_rx.await;
    });

    for i in 0..3 {
        queue.submit("a", json!(i), false).await.expect("submit");
    }
    shutdown_tx.send(()).expect("signal shutdown");
    handle.await.expect("worker join");
    assert_eq!(
        handler.seen_payloads().len(),
        3,
        "shutdown must drain the channel"
    );
}

// ---------------------------------------------------------------------------
// Durable (catalog-backed) tier, over the embedded SQLite catalog.
// ---------------------------------------------------------------------------

async fn sqlite_catalog() -> Arc<dyn pensieve_core::catalog::Catalog> {
    Arc::new(
        pensieve_catalog_sqlite::SqliteCatalog::connect_in_memory()
            .await
            .expect("open in-memory catalog"),
    )
}

#[tokio::test]
async fn durable_submit_survives_and_completes() {
    let catalog = sqlite_catalog().await;
    let handler = TestHandler::new();
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::mem::forget(_tx);
    let (queue, _h) = spawn(
        test_cfg("t_durable"),
        Some(catalog.clone()),
        handler.clone(),
        async move {
            let _ = rx.await;
        },
    );

    queue
        .submit("a", json!("keep-me"), true)
        .await
        .expect("submit");
    assert!(queue.barrier(&["a".into()]).await);
    assert_eq!(handler.seen_payloads(), vec![json!("keep-me")]);

    // The backing row was completed — nothing left to claim.
    let leftover = catalog
        .claim_task(
            "t_durable",
            pensieve_core::types::NodeId::new(),
            chrono::Duration::seconds(60),
        )
        .await
        .expect("claim");
    assert!(
        leftover.is_none(),
        "completed durable job must not be re-claimable"
    );
}

#[tokio::test]
async fn durable_jobs_replay_after_crash() {
    let catalog = sqlite_catalog().await;
    // Simulate a prior process that persisted a job but died before flushing.
    catalog
        .submit_task(
            "t_replay",
            None,
            json!({"partition": "a", "payload": "from-the-grave"}),
            0,
        )
        .await
        .expect("seed task");

    let handler = TestHandler::new();
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::mem::forget(_tx);
    let (_queue, _h) = spawn(
        test_cfg("t_replay"),
        Some(catalog),
        handler.clone(),
        async move {
            let _ = rx.await;
        },
    );

    assert!(
        eventually(Duration::from_secs(5), || {
            handler.seen_payloads() == vec![json!("from-the-grave")]
        })
        .await,
        "recovery must replay the orphaned durable job"
    );
}

#[tokio::test]
async fn failed_durable_job_requeues_via_store() {
    let catalog = sqlite_catalog().await;
    let handler = TestHandler::failing(u32::MAX);
    let mut cfg = test_cfg("t_requeue");
    cfg.max_retries = 0; // park to the store immediately
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::mem::forget(_tx);
    let (queue, _h) = spawn(cfg, Some(catalog), handler.clone(), async move {
        let _ = rx.await;
    });

    queue
        .submit("a", json!("poison"), true)
        .await
        .expect("submit");
    assert!(
        eventually(Duration::from_secs(5), || {
            handler.calls.load(Ordering::SeqCst) >= 2
        })
        .await,
        "a parked durable job must be re-claimed by the repair poll"
    );
}
