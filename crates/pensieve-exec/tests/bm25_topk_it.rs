//! Integration test for the BM25 lexical query path (`pensieve_exec::bm25_topk`).
//!
//! Exercises the full real plumbing against real components — a SQLite catalog
//! (`list_extents` / `list_index_sidecars`), an `InMemory` object store, the TLM
//! segment format (`open_extent` / `read_block`), and the disk `SidecarCache` —
//! the same trait surface `ann_topk_it` covers, here for the FTS sidecar:
//!
//! 1. write rows with a `body` text column into one extent and register it;
//! 2. build + upload + register the tantivy FTS sidecar (the same
//!    `TantivyFtsBuilder` the async `index_build` job uses);
//! 3. assert `bm25_topk` ranks the doc containing a rare term first and decodes
//!    its `RowAddress` correctly;
//! 4. assert that a table with **no** FTS sidecar returns `None` (caller falls
//!    back to its LIKE path).
//!
//! SQLite + `InMemory` deliberately: deterministic, no Docker, and it validates
//! the local/SQLite-mode contract. `bm25_topk` holds no backend-specific
//! assumptions, so the same path runs against Postgres + S3 in production.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray, TimestampNanosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use futures::StreamExt;
use pensieve_catalog_sqlite::SqliteCatalog;
use pensieve_core::catalog::{Catalog, ExtentManifest, SnapshotSummary, TableConfig, TableRef};
use pensieve_core::index_sidecar::{IndexSidecarDescriptor, SidecarBuilder, SidecarKind};
use pensieve_core::segment_format::{BlockPredicate, OpenExtentInput, SegmentFormat};
use pensieve_core::tenant::DEFAULT_TENANT;
use pensieve_core::types::TableId;
use pensieve_exec::bm25_topk;
use pensieve_format_tlm::TelemetryFormat;
use pensieve_index_fts::TantivyFtsBuilder;
use pensieve_storage::sidecar_cache::SidecarCache;
use object_store::path::Path as ObjPath;
use object_store::{memory::InMemory, ObjectStore};

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("id", DataType::Int64, true),
        Field::new("body", DataType::Utf8, true),
    ]))
}

fn make_batch(bodies: &[&str]) -> RecordBatch {
    let n = bodies.len();
    let ts = TimestampNanosecondArray::from((0..n).map(|i| i as i64).collect::<Vec<_>>());
    let ids = Int64Array::from((0..n).map(|i| i as i64).collect::<Vec<_>>());
    let body = StringArray::from(bodies.to_vec());
    RecordBatch::try_new(schema(), vec![Arc::new(ts), Arc::new(ids), Arc::new(body)]).unwrap()
}

async fn write_extent(
    catalog: &SqliteCatalog,
    format: &Arc<dyn SegmentFormat>,
    table_id: TableId,
    tref: &TableRef,
    batch: RecordBatch,
) -> ExtentManifest {
    let mut writer = format
        .start_extent(schema(), 64 * 1024 * 1024)
        .await
        .unwrap();
    writer.append(batch).await.unwrap();
    let res = writer.finish().await.unwrap();
    let manifest = ExtentManifest {
        id: res.extent_id,
        table_id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: res.object_path.clone(),
        byte_size: res.byte_size,
        row_count: res.row_count,
        min_timestamp: res.min_timestamp_nanos.and_then(|n| {
            chrono::DateTime::from_timestamp(n / 1_000_000_000, (n % 1_000_000_000) as u32)
        }),
        max_timestamp: res.max_timestamp_nanos.and_then(|n| {
            chrono::DateTime::from_timestamp(n / 1_000_000_000, (n % 1_000_000_000) as u32)
        }),
        column_stats: res.column_stats.clone(),
        present_paths: res.present_paths.clone(),
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    };
    let mut txn = catalog.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(manifest.clone()).await.unwrap();
    txn.commit(SnapshotSummary {
        rows_added: res.row_count as i64,
        operation: "ingest".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    manifest
}

async fn build_and_register_fts(
    catalog: &SqliteCatalog,
    format: &Arc<dyn SegmentFormat>,
    store: &Arc<dyn ObjectStore>,
    table_id: TableId,
    tref: &TableRef,
    manifest: &ExtentManifest,
    column: &str,
) {
    let reader = format
        .open_extent(OpenExtentInput {
            extent_id: manifest.id,
            table_id,
            schema: tref.schema.clone(),
            object_path: manifest.object_path.clone(),
            byte_size: manifest.byte_size,
        })
        .await
        .unwrap();
    let blocks = reader.pruned_blocks(&BlockPredicate::All).await.unwrap();
    let r = reader.clone();
    let batches = futures::stream::iter(blocks)
        .then(move |bid| {
            let r = r.clone();
            async move { r.read_block(bid, &[]).await }
        })
        .boxed();

    let built = TantivyFtsBuilder::new()
        .build(manifest, column, batches)
        .await
        .unwrap();
    let path = format!(
        "{DEFAULT_TENANT}/indexes/{}/{column}.{}",
        manifest.id,
        SidecarKind::TantivyFts.as_str()
    );
    let byte_size = built.bytes.len() as u64;
    store
        .put(&ObjPath::from(path.as_str()), built.bytes.into())
        .await
        .unwrap();
    catalog
        .register_index_sidecar(
            DEFAULT_TENANT,
            &IndexSidecarDescriptor {
                id: uuid::Uuid::new_v4(),
                extent_id: manifest.id,
                table_id,
                column: column.to_string(),
                kind: SidecarKind::TantivyFts,
                object_path: path,
                byte_size,
                params: built.params,
                embedding_model_id: None,
                created_at: chrono::Utc::now(),
            },
        )
        .await
        .unwrap();
}

async fn setup() -> (
    Arc<dyn Catalog>,
    Arc<dyn ObjectStore>,
    Arc<dyn SegmentFormat>,
    TableRef,
    SqliteCatalog,
    SidecarCache,
) {
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "pensieve"));
    let catalog = SqliteCatalog::connect_in_memory().await.unwrap();
    let db = catalog.create_database("default").await.unwrap();
    catalog
        .create_table(db, "logs", schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "logs").await.unwrap();
    let cache_root = std::env::temp_dir().join(format!(
        "pensieve-fts-it-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let cache = SidecarCache::new(&cache_root, 64 * 1024 * 1024);
    (
        Arc::new(catalog.clone()),
        store,
        format,
        tref,
        catalog,
        cache,
    )
}

#[tokio::test]
async fn bm25_topk_ranks_rare_term_and_decodes_addr() {
    let (cat, store, format, tref, sqlite, cache) = setup().await;

    let mut bodies: Vec<String> = (0..120)
        .map(|i| format!("common log line {i} status ok"))
        .collect();
    bodies[77] = "common log line OutOfMemoryError crash".to_string();
    let body_refs: Vec<&str> = bodies.iter().map(|s| s.as_str()).collect();
    let manifest = write_extent(&sqlite, &format, tref.id, &tref, make_batch(&body_refs)).await;
    build_and_register_fts(&sqlite, &format, &store, tref.id, &tref, &manifest, "body").await;

    let hits = bm25_topk(
        &cat,
        DEFAULT_TENANT,
        &store,
        &cache,
        &tref,
        "body",
        "outofmemoryerror",
        10,
        None,
    )
    .await
    .unwrap()
    .expect("sidecar exists → Some");
    assert!(!hits.is_empty(), "rare term must be found");
    // Row 77 (block 0) is the only doc with the term → ranks first.
    assert_eq!(hits[0].addr.row, 77, "rare-term doc ranks first");
    assert_eq!(hits[0].addr.block.0, 0);
    assert!(hits[0].score > 0.0);
}

#[tokio::test]
async fn bm25_topk_returns_none_without_sidecar() {
    let (cat, store, format, tref, sqlite, cache) = setup().await;
    // Write an extent but DON'T build a sidecar.
    let _m = write_extent(
        &sqlite,
        &format,
        tref.id,
        &tref,
        make_batch(&["hello world", "foo bar"]),
    )
    .await;

    let res = bm25_topk(
        &cat,
        DEFAULT_TENANT,
        &store,
        &cache,
        &tref,
        "body",
        "hello",
        10,
        None,
    )
    .await
    .unwrap();
    assert!(
        res.is_none(),
        "no FTS sidecar → None so caller uses LIKE fallback"
    );
}
