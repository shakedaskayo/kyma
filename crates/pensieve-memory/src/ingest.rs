//! Async-by-default memory ingestion.
//!
//! [`MemoryQueue`] is the typed facade write paths enqueue into: callers get
//! their (real, final) memory UUID back immediately, while a single
//! background worker drains submit-ordered batches and lands them with
//! dependency-aware batching:
//!
//! 1. **one** embedding call for every queued `Create`/`SaveAs` in the batch,
//! 2. **one** node-table append (one extent + one snapshot commit),
//! 3. **one** edge-table append, strictly *after* the node commit —
//!    so an edge never lands before the node it references (node-before-edge),
//!    and FIFO submit order preserves create-before-mutate and topic-key
//!    latest-wins semantics.
//!
//! Consistency contract (see `pensieve-queue` for the mechanics):
//! - Partition = **realm**. Reads call [`MemoryQueue::barrier`] with their
//!   target realms and only wait for writes in those realms (bounded wait,
//!   no-op when idle) — read-your-own-writes per realm.
//! - `durable = true` ops are persisted to the catalog's background task
//!   store before the submit returns and replay after a crash; row timestamps
//!   are stamped at *enqueue* time so replays keep latest-wins ordering.
//! - Redaction happens at enqueue, before anything touches any store.

use std::future::Future;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use pensieve_core::catalog::Catalog;
use pensieve_core::segment_format::SegmentFormat;
use pensieve_core::types::NodeId;
use pensieve_embed::EmbeddingBackend;
use pensieve_queue::{BatchHandler, Job, Queue, QueueConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::error;
use uuid::Uuid;

use crate::error::{MemoryError, Result};
use crate::types::CreateMemory;
use crate::writer::{redact_create, MemoryWriter};
use crate::{rows, EDGE_REFERENCES};

/// One queued memory mutation. Serialized as the queue payload (and into the
/// durable job store for `durable` submits), so it must stay self-contained:
/// `now` is stamped at enqueue time and used for every row timestamp the op
/// produces — replays and retries keep their original latest-wins position.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MemoryOp {
    /// Brand-new memory: node row + one `REFERENCES` edge per reference.
    Create {
        id: Uuid,
        mem: CreateMemory,
        now: String,
    },
    /// New version of an existing memory id (topic-key / entity upsert).
    SaveAs {
        id: Uuid,
        mem: CreateMemory,
        now: String,
    },
    /// Pre-built node row (status/importance/merge/judge mutations).
    AppendNode { row: Value },
    /// Pre-built edge row (links, RELATES_TO, INVALIDATES, MERGED_INTO, …).
    AddEdge { row: Value },
}

/// Tuning + durability tier for the memory ingest queue.
#[derive(Debug, Clone)]
pub struct MemoryIngestConfig {
    pub queue: QueueConfig,
    /// Persist explicit saves to the catalog's background-task store before
    /// acking (crash-safe). In-memory otherwise — loss window bounded by the
    /// linger interval.
    pub durable: bool,
}

impl MemoryIngestConfig {
    /// Read `PENSIEVE_MEMORY_QUEUE_*` tuning plus `PENSIEVE_MEMORY_QUEUE_DURABLE`
    /// (falling back to `durable_default`: `true` for the server binary,
    /// `false` for local/stdio).
    pub fn from_env(durable_default: bool) -> Self {
        let durable = std::env::var("PENSIEVE_MEMORY_QUEUE_DURABLE")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(durable_default);
        Self {
            queue: QueueConfig::from_env("memory_ops", "PENSIEVE_MEMORY_QUEUE"),
            durable,
        }
    }
}

/// Cheap-to-clone handle for enqueueing memory ops and awaiting realm
/// barriers. Lives on the shared tool context; `None` there means "use the
/// synchronous writer path".
#[derive(Clone)]
pub struct MemoryQueue {
    q: Queue,
}

impl MemoryQueue {
    /// Queue a brand-new memory. Redacts + stamps the row timestamp now;
    /// returns the final memory UUID immediately.
    pub async fn submit_create(&self, mem: &CreateMemory, durable: bool) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let red = redact_create(mem);
        let op = MemoryOp::Create {
            id,
            now: now_rfc3339(),
            mem: red,
        };
        self.submit(&mem.realm, op, durable).await?;
        Ok(id)
    }

    /// Queue a new version of an existing memory id (upsert).
    pub async fn submit_save_as(&self, id: Uuid, mem: &CreateMemory, durable: bool) -> Result<()> {
        let red = redact_create(mem);
        let op = MemoryOp::SaveAs {
            id,
            now: now_rfc3339(),
            mem: red,
        };
        self.submit(&mem.realm, op, durable).await
    }

    /// Queue a pre-built node row (read-modify-write mutations).
    pub async fn submit_node_row(&self, realm: &str, row: Value, durable: bool) -> Result<()> {
        self.submit(realm, MemoryOp::AppendNode { row }, durable)
            .await
    }

    /// Queue a pre-built edge row.
    pub async fn submit_edge_row(&self, realm: &str, row: Value, durable: bool) -> Result<()> {
        self.submit(realm, MemoryOp::AddEdge { row }, durable).await
    }

    async fn submit(&self, realm: &str, op: MemoryOp, durable: bool) -> Result<()> {
        let payload = serde_json::to_value(&op).map_err(|e| MemoryError::Queue(e.to_string()))?;
        self.q
            .submit(realm, payload, durable)
            .await
            .map(|_seq| ())
            .map_err(|e| MemoryError::Queue(e.to_string()))
    }

    /// Wait (bounded) until every op queued to the given realms before this
    /// call has been attempted — the read-your-own-writes barrier. An empty
    /// slice means all realms. No-op when nothing is pending.
    pub async fn barrier(&self, realms: &[String]) -> bool {
        self.q.barrier(realms).await
    }

    /// Flush everything (shutdown / stdio EOF). Returns `false` if pending
    /// ops could not be landed within `timeout`.
    pub async fn drain(&self, timeout: Duration) -> bool {
        self.q.drain(timeout).await
    }
}

/// Build the memory ingest queue and spawn its worker.
///
/// `node_id` identifies this worker's claims in the durable store; when
/// `cfg.durable` is set it MUST be a `node_id` already registered in the
/// catalog (e.g. the id returned by `Catalog::register_node`) — see
/// `pensieve_queue::spawn` for why. Callers with no node registration of their
/// own (no durable catalog, or a local/embedded catalog with no cluster
/// membership concept) can pass a fresh `NodeId::new()`.
///
/// The returned [`MemoryQueue`] is the submit/barrier handle; the
/// `JoinHandle` resolves once `shutdown` fires *and* the worker has drained —
/// await it before process exit so queued memories land.
pub fn spawn_memory_queue(
    catalog: Arc<dyn Catalog>,
    format: Arc<dyn SegmentFormat>,
    embed: Arc<dyn EmbeddingBackend>,
    cfg: MemoryIngestConfig,
    node_id: NodeId,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> (MemoryQueue, tokio::task::JoinHandle<()>) {
    // Warm the embedding backend off the hot path: the first barrier after a
    // save shouldn't pay the ONNX cold start on top of the flush itself.
    {
        let embed = embed.clone();
        tokio::spawn(async move {
            let _ = embed.embed(&["pensieve memory warmup".to_string()]).await;
        });
    }
    let writer = MemoryWriter::new(catalog.clone(), format, embed);
    let handler = Arc::new(BatchIngestor { writer });
    let durable = cfg.durable.then_some(catalog);
    let (q, handle) = pensieve_queue::spawn(cfg.queue, durable, handler, node_id, shutdown);
    (MemoryQueue { q }, handle)
}

/// The queue's batch handler: parses ops, embeds once, lands nodes then edges
/// in two bulk appends.
struct BatchIngestor {
    writer: MemoryWriter,
}

#[async_trait::async_trait]
impl BatchHandler for BatchIngestor {
    async fn handle(&self, jobs: &[Job]) -> anyhow::Result<()> {
        // Unparseable payloads are dropped with an error rather than failing
        // the batch — one poison op must not park its neighbours.
        let mut ops: Vec<MemoryOp> = Vec::with_capacity(jobs.len());
        for job in jobs {
            match serde_json::from_value::<MemoryOp>(job.payload.clone()) {
                Ok(op) => ops.push(op),
                Err(e) => error!(seq = job.seq, error = %e, "unparseable memory op; dropping"),
            }
        }
        if ops.is_empty() {
            return Ok(());
        }
        self.writer.ensure_provisioned().await?;

        // Phase 1 — one embedding call for every Create/SaveAs in the batch.
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|op| match op {
                MemoryOp::Create { mem, .. } | MemoryOp::SaveAs { mem, .. } => {
                    Some(mem.content.clone())
                }
                _ => None,
            })
            .collect();
        let vectors = self.writer.embed_batch(&texts).await?;

        // Phase 2 — build all rows in submit order.
        let mut node_rows: Vec<Value> = Vec::new();
        let mut edge_rows: Vec<Value> = Vec::new();
        let mut vi = 0usize;
        for op in &ops {
            match op {
                MemoryOp::Create { id, mem, now } => {
                    node_rows.push(rows::node_row(id, mem, &vectors[vi], now));
                    vi += 1;
                    let src = rows::node_id(id);
                    for r in &mem.references {
                        edge_rows.push(rows::edge_row(
                            &src,
                            r,
                            EDGE_REFERENCES,
                            &mem.realm,
                            None,
                            None,
                            now,
                        ));
                    }
                }
                MemoryOp::SaveAs { id, mem, now } => {
                    node_rows.push(rows::node_row(id, mem, &vectors[vi], now));
                    vi += 1;
                }
                MemoryOp::AppendNode { row } => node_rows.push(row.clone()),
                MemoryOp::AddEdge { row } => edge_rows.push(row.clone()),
            }
        }

        // Phase 3 — nodes first, then edges, each as ONE append (one extent +
        // one snapshot commit per table per flush). The idempotency key makes
        // an in-process batch retry after a partial success (nodes committed,
        // edges failed) skip the node re-append instead of double-writing.
        let key = batch_key(&node_rows, &edge_rows);
        if !node_rows.is_empty() {
            self.writer
                .append_node_rows_keyed(node_rows, Some(&format!("memops:nodes:{key}")))
                .await?;
        }
        if !edge_rows.is_empty() {
            self.writer
                .append_edge_rows_keyed(edge_rows, Some(&format!("memops:edges:{key}")))
                .await?;
        }
        Ok(())
    }
}

/// Stable-within-process fingerprint of a flush batch, used as the ingest
/// idempotency key so in-process retries don't double-apply the half of the
/// flush that already committed.
fn batch_key(node_rows: &[Value], edge_rows: &[Value]) -> String {
    let mut h = std::hash::DefaultHasher::new();
    for row in node_rows.iter().chain(edge_rows) {
        if let Some(id) = row.get("id").and_then(Value::as_str) {
            id.hash(&mut h);
        }
        for ts_col in ["updated_at", "created_at"] {
            if let Some(ts) = row.get(ts_col).and_then(Value::as_str) {
                ts.hash(&mut h);
            }
        }
    }
    (node_rows.len(), edge_rows.len()).hash(&mut h);
    format!("{:016x}", h.finish())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests;
