//! Integration tests for `ArtifactContentIndexer`: provision-on-demand,
//! text extraction + chunk/embed, idempotent re-index, and skip rules —
//! against the in-memory SQLite catalog + a local object store, with a
//! deterministic mock embedder (no ONNX download in CI).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use chrono::Utc;
use kyma_artifact_graph::content::{ArtifactContentIndexer, CHUNKS_TABLE};
use kyma_artifact_graph::ARTIFACTS_DB;
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use kyma_core::tenant::TenantId;
use kyma_embed::{EmbedError, EmbeddingBackend};
use kyma_format_tlm::TelemetryFormat;
use kyma_storage::{build_object_store, StorageConfig};
use object_store::path::Path as ObjPath;
use object_store::ObjectStore;
use uuid::Uuid;

#[derive(Debug)]
struct MockEmbed {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/4d"
    }
    fn dimension(&self) -> u16 {
        4
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(texts
            .iter()
            .map(|t| {
                let h = t.len() as f32;
                vec![h, 1.0, 0.0, 0.0]
            })
            .collect())
    }
}

struct Harness {
    catalog: Arc<dyn Catalog>,
    store: Arc<dyn ObjectStore>,
    embed: Arc<MockEmbed>,
    indexer: ArtifactContentIndexer,
    format: Arc<dyn SegmentFormat>,
}

async fn harness() -> Harness {
    let catalog: Arc<dyn Catalog> = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect_in_memory().await.unwrap(),
    );
    let tmp = std::env::temp_dir().join(format!("kyma-ac-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = build_object_store(&StorageConfig::Local {
        root: tmp.to_string_lossy().to_string(),
    })
    .unwrap();
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "test"));
    let embed = Arc::new(MockEmbed { calls: AtomicUsize::new(0) });
    let indexer = ArtifactContentIndexer::new(
        catalog.clone(),
        format.clone(),
        store.clone(),
        embed.clone(),
    );
    Harness { catalog, store, embed, indexer, format }
}

fn rec(source: &str, class: &str, path: &str, size: i64) -> ArtifactRecord {
    ArtifactRecord {
        id: Some(Uuid::new_v4()),
        tenant_id: TenantId::from_uuid(Uuid::nil()),
        object_path: path.into(),
        source: source.into(),
        artifact_class: class.into(),
        table_ref: None,
        connector_id: None,
        size_bytes: size,
        sha256: Some(format!("sha-{path}")),
        created_at: Some(Utc::now()),
        expires_at: None,
        deleted_at: None,
    }
}

async fn put(store: &Arc<dyn ObjectStore>, path: &str, bytes: &[u8]) {
    kyma_storage::put_artifact(store, &ObjPath::from(path), Bytes::copy_from_slice(bytes))
        .await
        .unwrap();
}

/// Count rows currently scannable in `artifacts.artifact_chunks`.
async fn chunk_count(h: &Harness) -> usize {
    use datafusion::prelude::SessionContext;
    let tref = h.catalog.lookup_table(ARTIFACTS_DB, CHUNKS_TABLE).await.unwrap();
    let kt = Arc::new(kyma_exec::KymaTable::new(
        tref,
        h.catalog.clone(),
        h.format.clone(),
    ));
    let ctx = SessionContext::new();
    ctx.register_table(CHUNKS_TABLE, kt).unwrap();
    let df = ctx
        .sql(&format!("SELECT COUNT(*) AS n FROM {CHUNKS_TABLE}"))
        .await
        .unwrap();
    let batches = df.collect().await.unwrap();
    let col = batches[0].column(0);
    let arr = col.as_any().downcast_ref::<arrow_array::Int64Array>().unwrap();
    arr.value(0) as usize
}

#[tokio::test]
async fn indexes_text_artifact_and_is_idempotent() {
    let h = harness().await;
    let r = rec("fswatch", "log", "artifacts/t/a.log", 64);
    put(&h.store, &r.object_path, b"error: connection refused to db-7\nretrying in 5s\n").await;

    let n = h.indexer.index(std::slice::from_ref(&r)).await.unwrap();
    assert_eq!(n, 1, "short log fits one chunk");
    assert_eq!(chunk_count(&h).await, 1);
    assert_eq!(h.embed.calls.load(Ordering::SeqCst), 1);

    // Re-index: ledger hit BEFORE the blob fetch — no new rows, no new embed call.
    let again = h.indexer.index(std::slice::from_ref(&r)).await.unwrap();
    assert_eq!(again, 0, "second sweep is a no-op");
    assert_eq!(chunk_count(&h).await, 1);
    assert_eq!(h.embed.calls.load(Ordering::SeqCst), 1, "no re-embed on re-sync");
}

#[tokio::test]
async fn changed_content_hash_reindexes() {
    let h = harness().await;
    let mut r = rec("fswatch", "log", "artifacts/t/grow.log", 64);
    put(&h.store, &r.object_path, b"v1 line\n").await;
    assert_eq!(h.indexer.index(std::slice::from_ref(&r)).await.unwrap(), 1);

    // Same artifact path, new content hash ⇒ new idempotency key ⇒ re-indexed.
    put(&h.store, &r.object_path, b"v2 line with more text\n").await;
    r.sha256 = Some("sha-v2".into());
    assert_eq!(h.indexer.index(std::slice::from_ref(&r)).await.unwrap(), 1);
    assert_eq!(chunk_count(&h).await, 2);
}

#[tokio::test]
async fn skips_binary_nontext_deleted_producer_attached_and_oversized() {
    let h = harness().await;

    let bin = rec("fswatch", "log", "artifacts/t/bin.log", 6);
    put(&h.store, &bin.object_path, b"\x00\x01\x02\x03\x04\x05").await;

    let img = rec("agent", "image", "artifacts/t/pic.png", 4);
    put(&h.store, &img.object_path, b"PNG!").await;

    let mut gone = rec("fswatch", "log", "artifacts/t/gone.log", 4);
    gone.deleted_at = Some(Utc::now());
    put(&h.store, &gone.object_path, b"text").await;

    let gh = rec("github", "log", "artifacts/t/ci.log", 4);
    put(&h.store, &gh.object_path, b"text").await;

    let big = rec("fswatch", "log", "artifacts/t/big.log", 100 * 1024 * 1024);

    let missing_blob = rec("fswatch", "log", "artifacts/t/missing.log", 4);

    let n = h
        .indexer
        .index(&[bin, img, gone, gh, big, missing_blob])
        .await
        .unwrap();
    assert_eq!(n, 0, "every record hits a skip rule");
    assert_eq!(h.embed.calls.load(Ordering::SeqCst), 0, "skips never embed");
}

#[tokio::test]
async fn long_text_splits_into_multiple_searchable_chunks() {
    let h = harness().await;
    let r = rec("agent", "file", "artifacts/t/long.txt", 9000);
    let text = (0..120)
        .map(|i| format!("paragraph {i}: {}", "lorem ipsum dolor ".repeat(4)))
        .collect::<Vec<_>>()
        .join("\n");
    put(&h.store, &r.object_path, text.as_bytes()).await;

    let n = h.indexer.index(std::slice::from_ref(&r)).await.unwrap();
    assert!(n > 1, "long file must produce multiple chunks, got {n}");
    assert_eq!(chunk_count(&h).await, n);
}
