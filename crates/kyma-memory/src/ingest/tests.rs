//! Batch-ingest pipeline coverage over the embedded SQLite catalog + local
//! filesystem format: one embed call per flush, one extent per table per
//! flush, node-before-edge, FIFO latest-wins, realm-scoped barriers.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kyma_core::catalog::{Catalog, PrunePredicate};
use kyma_embed::{EmbedError, EmbeddingBackend};
use kyma_format_tlm::TelemetryFormat;
use kyma_storage::{build_object_store, StorageConfig};

use super::{spawn_memory_queue, MemoryIngestConfig, MemoryQueue};
use crate::types::CreateMemory;
use crate::{EDGE_TABLE, NODE_TABLE};

/// Deterministic counting embedder. The off-hot-path warmup call (a single
/// known sentinel text) is not counted as a batch.
#[derive(Debug)]
struct MockEmbed {
    batch_sizes: Mutex<Vec<usize>>,
    calls: AtomicU32,
}

impl MockEmbed {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            batch_sizes: Mutex::new(Vec::new()),
            calls: AtomicU32::new(0),
        })
    }
}

#[async_trait::async_trait]
impl EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/test"
    }

    fn dimension(&self) -> u16 {
        4
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let is_warmup = texts.len() == 1 && texts[0] == "kyma memory warmup";
        if !is_warmup {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.batch_sizes.lock().unwrap().push(texts.len());
        }
        Ok(texts
            .iter()
            .map(|t| {
                let f = t.len() as f32;
                vec![f, f * 0.5, f * 0.25, 1.0]
            })
            .collect())
    }
}

struct TestRig {
    catalog: Arc<dyn Catalog>,
    embed: Arc<MockEmbed>,
    queue: MemoryQueue,
    _tmp: std::path::PathBuf,
}

async fn rig(durable: bool) -> TestRig {
    let catalog: Arc<dyn Catalog> = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect_in_memory()
            .await
            .expect("open in-memory catalog"),
    );
    let tmp = std::env::temp_dir().join(format!("kyma-mem-ingest-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).expect("tmp data dir");
    let store = build_object_store(&StorageConfig::Local {
        root: tmp.to_string_lossy().to_string(),
    })
    .expect("local store");
    let format: Arc<dyn kyma_core::segment_format::SegmentFormat> =
        Arc::new(TelemetryFormat::new(store, "test"));
    let embed = MockEmbed::new();

    let cfg = MemoryIngestConfig {
        queue: kyma_queue::QueueConfig {
            name: "memory_ops_test".into(),
            max_batch: 64,
            linger: Duration::from_millis(30),
            channel_cap: 256,
            max_retries: 1,
            barrier_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_millis(100),
        },
        durable,
    };
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::mem::forget(_tx);
    let (queue, _handle) =
        spawn_memory_queue(catalog.clone(), format, embed.clone(), cfg, async move {
            let _ = rx.await;
        });
    TestRig {
        catalog,
        embed,
        queue,
        _tmp: tmp,
    }
}

async fn table_rows_and_extents(catalog: &Arc<dyn Catalog>, table: &str) -> (u64, usize) {
    let tref = catalog
        .lookup_table(crate::DEFAULT_DATABASE, table)
        .await
        .expect("table exists");
    let extents = catalog
        .list_extents(
            tref.id,
            tref.current_snapshot_id,
            &PrunePredicate::default(),
        )
        .await
        .expect("list extents");
    let rows = extents.iter().map(|e| e.row_count).sum();
    (rows, extents.len())
}

#[tokio::test]
async fn coalesced_flush_embeds_once_and_commits_once_per_table() {
    let rig = rig(false).await;

    // 5 saves across 2 realms; 2 carry references (edges).
    let mut ids = Vec::new();
    for i in 0..5 {
        let mut m = CreateMemory::new(format!("memory number {i}"));
        m.realm = if i % 2 == 0 {
            "alpha".into()
        } else {
            "beta".into()
        };
        if i < 2 {
            m.references = vec![format!("default::thing:{i}")];
        }
        ids.push(rig.queue.submit_create(&m, false).await.expect("submit"));
    }
    assert!(rig.queue.barrier(&[]).await, "barrier must flush the batch");

    // One embedding round-trip for all five contents.
    assert_eq!(
        rig.embed.calls.load(Ordering::SeqCst),
        1,
        "expected ONE batched embed call"
    );
    assert_eq!(*rig.embed.batch_sizes.lock().unwrap(), vec![5]);

    // One extent + one commit per table for the whole flush.
    let (node_rows, node_extents) = table_rows_and_extents(&rig.catalog, NODE_TABLE).await;
    assert_eq!(
        (node_rows, node_extents),
        (5, 1),
        "5 node rows in ONE extent"
    );
    let (edge_rows, edge_extents) = table_rows_and_extents(&rig.catalog, EDGE_TABLE).await;
    assert_eq!(
        (edge_rows, edge_extents),
        (2, 1),
        "2 edge rows in ONE extent"
    );
}

#[tokio::test]
async fn fifo_same_id_versions_keep_submit_order() {
    let rig = rig(false).await;

    let mut first = CreateMemory::new("version one");
    first.realm = "r".into();
    let id = rig
        .queue
        .submit_create(&first, false)
        .await
        .expect("submit");

    let mut second = CreateMemory::new("version two");
    second.realm = "r".into();
    rig.queue
        .submit_save_as(id, &second, false)
        .await
        .expect("submit save_as");

    assert!(rig.queue.barrier(&["r".into()]).await);
    let (rows, _) = table_rows_and_extents(&rig.catalog, NODE_TABLE).await;
    assert_eq!(
        rows, 2,
        "append-only: both versions stored; latest-wins resolves at read"
    );
    // Timestamps are stamped at enqueue: the upsert sorts after the create.
    // (Read-path latest-wins is covered by the server-side recall tests.)
}

#[tokio::test]
async fn realm_scoped_barrier_and_edge_only_ops() {
    let rig = rig(false).await;

    let mut m = CreateMemory::new("a memory to link");
    m.realm = "linked".into();
    let id = rig.queue.submit_create(&m, false).await.expect("submit");

    // Queue an edge to the not-yet-flushed memory — FIFO + node-before-edge
    // makes this safe within the same flush.
    let now = chrono::Utc::now().to_rfc3339();
    let edge = crate::rows::edge_row(
        &crate::rows::node_id(&id),
        "default::entity:x",
        crate::EDGE_RESOLVES_TO,
        "linked",
        Some("default"),
        None,
        &now,
    );
    rig.queue
        .submit_edge_row("linked", edge, false)
        .await
        .expect("submit edge");

    // A barrier on an untouched realm never waits on "linked"'s backlog.
    assert!(rig.queue.barrier(&["untouched".into()]).await);
    assert!(rig.queue.barrier(&["linked".into()]).await);

    let (node_rows, _) = table_rows_and_extents(&rig.catalog, NODE_TABLE).await;
    let (edge_rows, _) = table_rows_and_extents(&rig.catalog, EDGE_TABLE).await;
    assert_eq!((node_rows, edge_rows), (1, 1));
}

#[tokio::test]
async fn durable_ops_replay_after_simulated_crash() {
    // Build a rig whose worker we shut down BEFORE it can flush, leaving the
    // durable rows orphaned in the catalog store — then start a second worker
    // over the same catalog and assert the ops land.
    let catalog: Arc<dyn Catalog> = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect_in_memory()
            .await
            .expect("open in-memory catalog"),
    );
    let tmp = std::env::temp_dir().join(format!("kyma-mem-replay-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).expect("tmp data dir");
    let store = build_object_store(&StorageConfig::Local {
        root: tmp.to_string_lossy().to_string(),
    })
    .expect("local store");
    let format: Arc<dyn kyma_core::segment_format::SegmentFormat> =
        Arc::new(TelemetryFormat::new(store, "test"));

    // "Crashed" process: persist the durable op directly to the store the way
    // a submit does, without any worker running to consume it.
    let mut m = CreateMemory::new("survives the crash");
    m.realm = "dur".into();
    let id = uuid::Uuid::new_v4();
    let op = super::MemoryOp::Create {
        id,
        mem: m,
        now: chrono::Utc::now().to_rfc3339(),
    };
    catalog
        .submit_task(
            "memory_ops_test",
            None,
            serde_json::json!({"partition": "dur", "payload": serde_json::to_value(&op).unwrap()}),
            0,
        )
        .await
        .expect("seed durable op");

    // Second process: spawn the worker; recovery must replay the op.
    let embed = MockEmbed::new();
    let cfg = MemoryIngestConfig {
        queue: kyma_queue::QueueConfig {
            name: "memory_ops_test".into(),
            max_batch: 64,
            linger: Duration::from_millis(10),
            channel_cap: 64,
            max_retries: 1,
            barrier_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_millis(50),
        },
        durable: true,
    };
    let (_tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::mem::forget(_tx);
    let (_queue, _handle) = spawn_memory_queue(catalog.clone(), format, embed, cfg, async move {
        let _ = rx.await;
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(tref) = catalog
            .lookup_table(crate::DEFAULT_DATABASE, NODE_TABLE)
            .await
        {
            let extents = catalog
                .list_extents(
                    tref.id,
                    tref.current_snapshot_id,
                    &PrunePredicate::default(),
                )
                .await
                .unwrap_or_default();
            if extents.iter().map(|e| e.row_count).sum::<u64>() == 1 {
                return; // replayed and landed
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "durable op was not replayed within 5s"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
