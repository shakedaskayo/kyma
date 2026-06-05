//! Async-by-default memory writes, end to end through the MCP tool dispatch:
//! save tools ack immediately with `queued: true`, the background worker lands
//! batched embeds + group commits, `flush_memory` is the commit barrier, and
//! the rows (with writer provenance) are visible via `run_sql`.
//!
//! DB-free: embedded SQLite catalog + local-fs format + a mock embedder.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kyma_mcp::ToolDispatch;
use kyma_server::agent::SharedToolCtx;
use serde_json::{json, Value};

#[derive(Debug)]
struct MockEmbed {
    calls: AtomicU32,
    batch_sizes: Mutex<Vec<usize>>,
}

#[async_trait::async_trait]
impl kyma_embed::EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/e2e"
    }
    fn dimension(&self) -> u16 {
        4
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, kyma_embed::EmbedError> {
        if !(texts.len() == 1 && texts[0] == "kyma memory warmup") {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.batch_sizes.lock().unwrap().push(texts.len());
        }
        Ok(texts
            .iter()
            .map(|t| vec![t.len() as f32, 1.0, 2.0, 3.0])
            .collect())
    }
}

async fn dispatch_with_queue() -> (ToolDispatch, Arc<MockEmbed>) {
    let catalog: Arc<dyn kyma_core::catalog::Catalog> = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect_in_memory()
            .await
            .expect("in-memory catalog"),
    );
    let tmp = std::env::temp_dir().join(format!("kyma-mcp-asyncmem-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).expect("tmp dir");
    let store = kyma_storage::build_object_store(&kyma_storage::StorageConfig::Local {
        root: tmp.to_string_lossy().to_string(),
    })
    .expect("local store");
    let format: Arc<dyn kyma_core::segment_format::SegmentFormat> =
        Arc::new(kyma_format_tlm::TelemetryFormat::new(store, "test"));

    let embed = Arc::new(MockEmbed {
        calls: AtomicU32::new(0),
        batch_sizes: Mutex::new(Vec::new()),
    });
    let cfg = kyma_memory::MemoryIngestConfig {
        queue: kyma_queue::QueueConfig {
            name: "memory_ops_e2e".into(),
            max_batch: 64,
            linger: Duration::from_millis(25),
            channel_cap: 256,
            max_retries: 1,
            barrier_timeout: Duration::from_secs(10),
            poll_interval: Duration::from_millis(200),
        },
        // Durable tier over the embedded catalog — exercises the
        // background_tasks store end to end.
        durable: true,
    };
    let (_stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    std::mem::forget(_stop_tx);
    let (queue, _worker) = kyma_memory::spawn_memory_queue(
        catalog.clone(),
        format.clone(),
        embed.clone(),
        cfg,
        async move {
            let _ = stop_rx.await;
        },
    );

    let shared = SharedToolCtx {
        catalog,
        format,
        pool: None,
        memory: Some(queue),
    };
    (ToolDispatch::new(shared), embed)
}

fn structured(v: &Value) -> &Value {
    &v["structuredContent"]
}

#[tokio::test]
async fn queued_saves_batch_flush_and_become_visible() {
    let (dispatch, embed) = dispatch_with_queue().await;

    // 1. Bulk save: acks immediately with queued ids.
    let bulk = dispatch
        .call(
            "save_memories",
            json!({"memories": [
                {"content": "fact one about the system", "realm": "e2e"},
                {"content": "decision two about the system", "memory_type": "decision", "realm": "e2e"},
                {"content": "learning three about the system", "memory_type": "learning", "realm": "e2e",
                 "references": ["default::repo:kyma"]},
            ]}),
        )
        .await
        .expect("save_memories");
    let bulk = structured(&bulk);
    assert_eq!(bulk["saved"], json!(true));
    assert_eq!(bulk["queued"], json!(true));
    assert_eq!(bulk["ids"].as_array().unwrap().len(), 3);

    // 2. Single save with provenance-bearing identity.
    let single = dispatch
        .call(
            "save_memory",
            json!({"content": "fact four", "realm": "e2e"}),
        )
        .await
        .expect("save_memory");
    let single = structured(&single);
    assert_eq!(single["saved"], json!(true));
    assert_eq!(single["queued"], json!(true));
    assert!(single["id"].as_str().is_some());

    // 3. Explicit barrier: everything queued is committed afterwards.
    let flushed = dispatch
        .call("flush_memory", json!({"realms": ["e2e"]}))
        .await
        .expect("flush_memory");
    assert_eq!(structured(&flushed)["flushed"], json!(true));

    // 4. The four saves above were enqueued back-to-back → the worker embeds
    //    them in (at most two) batched calls, not four singles.
    let calls = embed.calls.load(Ordering::SeqCst);
    assert!(calls <= 2, "expected batched embedding, got {calls} calls");
    let total: usize = embed.batch_sizes.lock().unwrap().iter().sum();
    assert_eq!(total, 4, "all four contents embedded");

    // 5. Rows are visible via plain SQL: 4 nodes (+1 REFERENCES edge), with
    //    the writer identity stamped into provenance.
    let rows = dispatch
        .call(
            "run_sql",
            json!({"database": "memory",
                   "sql": "SELECT id, provenance FROM memory_nodes",
                   "limit": 50}),
        )
        .await
        .expect("run_sql nodes");
    let rows_dbg = structured(&rows).clone();
    let rows = rows_dbg["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("rows missing: {rows_dbg}"))
        .clone();
    assert_eq!(rows.len(), 4, "4 memory node rows committed");
    let provenance = rows[0]["provenance"].as_str().expect("provenance set");
    let prov: Value = serde_json::from_str(provenance).expect("provenance is JSON");
    assert!(
        prov["writer"]["host"].as_str().is_some(),
        "writer.host stamped"
    );

    let edges = dispatch
        .call(
            "run_sql",
            json!({"database": "memory",
                   "sql": "SELECT id FROM memory_edges",
                   "limit": 50}),
        )
        .await
        .expect("run_sql edges");
    let edges = structured(&edges)["rows"].as_array().expect("rows").clone();
    assert_eq!(edges.len(), 1, "one REFERENCES edge from the third memory");
}

#[tokio::test]
async fn read_your_own_writes_via_list_memories() {
    let (dispatch, _embed) = dispatch_with_queue().await;

    let saved = dispatch
        .call(
            "save_memory",
            json!({"content": "ryw check", "realm": "ryw"}),
        )
        .await
        .expect("save_memory");
    assert_eq!(structured(&saved)["queued"], json!(true));

    // No explicit flush: list_memories barriers on its target realm itself.
    let listed = dispatch
        .call("list_memories", json!({"realm": "ryw"}))
        .await
        .expect("list_memories");
    let rows = structured(&listed)["rows"]
        .as_array()
        .expect("rows")
        .clone();
    assert_eq!(
        rows.len(),
        1,
        "read-your-own-writes: just-queued save is visible"
    );
}

#[tokio::test]
async fn topic_key_upserts_stay_latest_wins_under_batching() {
    let (dispatch, _embed) = dispatch_with_queue().await;

    for (i, content) in ["v1 of the auth model", "v2 of the auth model"]
        .iter()
        .enumerate()
    {
        let out = dispatch
            .call(
                "save_memory",
                json!({"content": content, "realm": "tk", "topic_key": "architecture/auth"}),
            )
            .await
            .expect("save_memory");
        let out = structured(&out);
        assert_eq!(out["saved"], json!(true), "save #{i} acked");
    }
    let _ = dispatch
        .call("flush_memory", json!({}))
        .await
        .expect("flush");

    // Latest-wins: the second save updated the first in place.
    let listed = dispatch
        .call("list_memories", json!({"realm": "tk"}))
        .await
        .expect("list_memories");
    let rows = structured(&listed)["rows"]
        .as_array()
        .expect("rows")
        .clone();
    assert_eq!(rows.len(), 1, "topic-key upsert dedups to one memory");
    assert!(
        rows[0]["content_preview"]
            .as_str()
            .unwrap_or_default()
            .contains("v2"),
        "the later save wins"
    );
}
