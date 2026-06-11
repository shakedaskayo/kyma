//! Integration tests for the filedrop scan counters and the scan hook:
//! `tick()` returns per-cycle `FiledropScan` stats, and `run()` hands each
//! cycle's stats to the optional hook (the bridge kyma-bin uses to heartbeat
//! the watcher registry without this crate depending on kyma-datasources).

use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::Catalog;
use kyma_format_tlm::TelemetryFormat;
use kyma_ingest_core::WritePath;
use kyma_ingest_filedrop::{FiledropConfig, FiledropScan, FiledropWatcher};
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::ObjectStore;
use std::sync::Arc;
use std::time::Duration;

async fn watcher_fixture(config: FiledropConfig) -> (FiledropWatcher, Arc<InMemory>) {
    let catalog: Arc<dyn Catalog> = Arc::new(
        SqliteCatalog::connect_in_memory()
            .await
            .expect("sqlite catalog"),
    );
    let store = Arc::new(InMemory::new());
    let format = Arc::new(TelemetryFormat::new(store.clone(), "kyma-test"));
    let write_path = WritePath::new(catalog.clone(), format);
    let watcher = FiledropWatcher::new(catalog, store.clone(), write_path, config);
    (watcher, store)
}

async fn put(store: &InMemory, path: &str, body: &str) {
    store
        .put(&Path::from(path), body.as_bytes().to_vec().into())
        .await
        .expect("put object");
}

#[tokio::test]
async fn tick_counts_seen_processed_and_replays() {
    let (watcher, store) = watcher_fixture(FiledropConfig::default()).await;

    // Empty store: nothing seen, nothing processed.
    let scan = watcher.tick().await.expect("tick on empty store");
    assert_eq!(scan.seen, 0);
    assert_eq!(scan.processed, 0);
    assert_eq!(scan.errors, 0);

    put(&store, "ingest/db1/t1/a.ndjson", "{\"label\":\"x\",\"body\":\"one\"}\n").await;
    put(&store, "ingest/db1/t1/b.ndjson", "{\"label\":\"y\",\"body\":\"two\"}\n").await;

    let scan = watcher.tick().await.expect("tick with two files");
    assert_eq!(scan.seen, 2, "both dropped files listed");
    assert_eq!(scan.processed, 2, "both files newly ingested");
    assert_eq!(scan.errors, 0);

    // Second tick: files still there (delete_after_ingest=false), but the
    // idempotency ledger replays them — seen but not processed.
    let scan = watcher.tick().await.expect("replay tick");
    assert_eq!(scan.seen, 2);
    assert_eq!(scan.processed, 0, "ledger replays are not new work");
    assert_eq!(scan.errors, 0);
}

#[tokio::test]
async fn tick_counts_per_file_errors() {
    let (watcher, store) = watcher_fixture(FiledropConfig::default()).await;

    put(&store, "ingest/db1/t1/good.ndjson", "{\"label\":\"x\",\"body\":\"ok\"}\n").await;
    // Unsupported extension -> per-file error, scan keeps going.
    put(&store, "ingest/db1/t1/bad.parquet", "not parquet").await;

    let scan = watcher.tick().await.expect("tick");
    assert_eq!(scan.seen, 2);
    assert_eq!(scan.processed, 1);
    assert_eq!(scan.errors, 1, "unsupported extension counts as an error");
}

#[tokio::test]
async fn run_fires_scan_hook_each_cycle() {
    let config = FiledropConfig {
        poll_interval: Duration::from_millis(20),
        ..FiledropConfig::default()
    };
    let (watcher, store) = watcher_fixture(config).await;
    put(&store, "ingest/db1/t1/a.ndjson", "{\"label\":\"x\",\"body\":\"one\"}\n").await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<FiledropScan>();
    let watcher = watcher.with_scan_hook(Arc::new(move |scan| {
        let _ = tx.send(scan);
    }));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(watcher.run(async move {
        let _ = shutdown_rx.await;
    }));

    let first = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("hook fired within 5s")
        .expect("channel open");
    assert_eq!(first.seen, 1);
    assert_eq!(first.processed, 1);
    assert_eq!(first.errors, 0);
    assert!(first.duration_ms < 1000, "duration_ms sanity: timer must be wired");

    // A subsequent cycle replays from the ledger: seen but not processed.
    let second = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("hook fired again within 5s")
        .expect("channel open");
    assert_eq!(second.seen, 1);
    assert_eq!(second.processed, 0);

    let _ = shutdown_tx.send(());
    handle.await.expect("watcher task joins");
}
