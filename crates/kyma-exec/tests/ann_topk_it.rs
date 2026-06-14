//! Integration test for the ANN query path (`kyma_exec::ann_topk`).
//!
//! Exercises the *full* real pipeline end-to-end against real components:
//! a SQLite catalog (`list_extents` / `list_index_sidecars`), an `InMemory`
//! object store, the telemetry segment format (`open_extent` / `read_block`),
//! and the disk [`SidecarCache`] (`get_or_fetch`). We:
//!
//! 1. write N seeded, clustered vectors into one extent via the TLM writer and
//!    register it in the catalog;
//! 2. build the IVF+RaBitQ sidecar from that extent's blocks (the same
//!    [`IvfRabitqBuilder`] the async `index_build` job uses), upload it to the
//!    store at the conventional path, and register the descriptor;
//! 3. assert `ann_topk` matches a brute-force exact oracle at `recall@10 ≥
//!    0.95`;
//! 4. assert that an extent with **no** sidecar falls back to exact scan and
//!    returns the exact top-k (recall 1.0).
//!
//! Uses SQLite + `InMemory` deliberately: it is deterministic, needs no Docker,
//! and validates the local/SQLite-mode contract the engine must satisfy. The
//! same code path runs against Postgres + S3/MinIO in production — `ann_topk`
//! holds no backend-specific assumptions (it talks only to the `Catalog`,
//! `SegmentFormat`, `ObjectStore`, and `SidecarCache` traits).

use std::sync::Arc;

use arrow_array::builder::{FixedSizeListBuilder, Float32Builder};
use arrow_array::{Int64Array, RecordBatch, TimestampNanosecondArray};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use futures::StreamExt;
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::{Catalog, ExtentManifest, SnapshotSummary, TableConfig, TableRef};
use kyma_core::index_sidecar::{IndexSidecarDescriptor, SidecarBuilder, SidecarKind};
use kyma_core::segment_format::{BlockPredicate, OpenExtentInput, SegmentFormat};
use kyma_core::tenant::DEFAULT_TENANT;
use kyma_core::types::TableId;
use kyma_exec::{ann_topk, AnnParams};
use kyma_format_tlm::TelemetryFormat;
use kyma_index_vector::IvfRabitqBuilder;
use kyma_storage::sidecar_cache::SidecarCache;
use object_store::path::Path as ObjPath;
use object_store::{memory::InMemory, ObjectStore};

const DIM: usize = 64;

// --- deterministic RNG (SplitMix64), mirrors the recall test ---
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn gaussian(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }
}

fn norm(mut v: Vec<f32>) -> Vec<f32> {
    let n = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if n > 1e-12 {
        for x in &mut v {
            *x /= n;
        }
    }
    v
}

fn cos_dist(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    1.0 - dot
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("id", DataType::Int64, true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                DIM as i32,
            ),
            true,
        ),
    ]))
}

/// Build one RecordBatch of `vectors.len()` rows.
fn make_batch(vectors: &[Vec<f32>], id_base: i64, ts_base: i64) -> RecordBatch {
    let n = vectors.len();
    let ts = TimestampNanosecondArray::from((0..n).map(|i| ts_base + i as i64).collect::<Vec<_>>());
    let ids = Int64Array::from((0..n).map(|i| id_base + i as i64).collect::<Vec<_>>());
    let values = Float32Builder::new();
    let mut b = FixedSizeListBuilder::new(values, DIM as i32);
    for v in vectors {
        b.values().append_slice(v);
        b.append(true);
    }
    let arr = b.finish();
    RecordBatch::try_new(schema(), vec![Arc::new(ts), Arc::new(ids), Arc::new(arr)]).unwrap()
}

/// Write `batches` to a new extent via the TLM format and register it.
async fn write_extent(
    catalog: &SqliteCatalog,
    format: &Arc<dyn SegmentFormat>,
    table_id: TableId,
    tref: &TableRef,
    batches: Vec<RecordBatch>,
) -> ExtentManifest {
    let mut writer = format
        .start_extent(schema(), 64 * 1024 * 1024)
        .await
        .unwrap();
    for b in batches {
        writer.append(b).await.unwrap();
    }
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

/// Build + upload + register the IVF+RaBitQ sidecar for one extent, exactly as
/// the `index_build` job executor would.
async fn build_and_register_sidecar(
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

    let builder = IvfRabitqBuilder::new();
    let built = builder.build(manifest, column, batches).await.unwrap();

    let path = format!(
        "{DEFAULT_TENANT}/indexes/{}/{column}.{}",
        manifest.id,
        SidecarKind::IvfRabitq.as_str()
    );
    let byte_size = built.bytes.len() as u64;
    store
        .put(&ObjPath::from(path.as_str()), built.bytes.into())
        .await
        .unwrap();
    let desc = IndexSidecarDescriptor {
        id: uuid::Uuid::new_v4(),
        extent_id: manifest.id,
        table_id,
        column: column.to_string(),
        kind: SidecarKind::IvfRabitq,
        object_path: path,
        byte_size,
        params: built.params,
        embedding_model_id: built.embedding_model_id,
        created_at: chrono::Utc::now(),
    };
    catalog
        .register_index_sidecar(DEFAULT_TENANT, &desc)
        .await
        .unwrap();
}

/// Generate `n` clustered unit vectors around `n_clusters` anchors (seeded).
fn gen_clustered(n: usize, n_clusters: usize, seed: u64) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = Rng::new(seed);
    let anchors: Vec<Vec<f32>> = (0..n_clusters)
        .map(|_| norm((0..DIM).map(|_| rng.gaussian() as f32).collect()))
        .collect();
    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| {
            let a = &anchors[i % n_clusters];
            norm(a.iter().map(|c| c + 0.30 * rng.gaussian() as f32).collect())
        })
        .collect();
    (vectors, anchors)
}

/// Brute-force exact top-k row indices over `vectors`.
fn exact_topk(query: &[f32], vectors: &[Vec<f32>], k: usize) -> Vec<usize> {
    let mut scored: Vec<(usize, f32)> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| (i, cos_dist(query, v)))
        .collect();
    scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap().then(a.0.cmp(&b.0)));
    scored.into_iter().take(k).map(|(i, _)| i).collect()
}

/// Map an `AnnHit` (block, row over 1000-row blocks) back to a flat row index.
fn hit_to_flat_idx(block: u32, row: u32, rows_per_block: usize) -> usize {
    block as usize * rows_per_block + row as usize
}

async fn setup() -> (
    SqliteCatalog,
    Arc<dyn SegmentFormat>,
    Arc<dyn ObjectStore>,
    SidecarCache,
    std::path::PathBuf,
) {
    let catalog = SqliteCatalog::connect_in_memory()
        .await
        .expect("sqlite catalog");
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "kyma"));
    let cache_root = std::env::temp_dir().join(format!(
        "kyma-ann-it-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let cache = SidecarCache::new(&cache_root, 256 * 1024 * 1024);
    (catalog, format, store, cache, cache_root)
}

#[tokio::test]
async fn ann_topk_matches_oracle_with_sidecar() {
    let (catalog, format, store, cache, cache_root) = setup().await;
    let catalog_dyn: Arc<dyn Catalog> = Arc::new(catalog.clone());

    let db_id = catalog.create_database("default").await.unwrap();
    let table_id = catalog
        .create_table(db_id, "vecs", schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "vecs").await.unwrap();

    // 5000 rows in five 1000-row blocks (mirrors the builder's block convention).
    let n = 5_000;
    let rows_per_block = 1_000;
    let (vectors, anchors) = gen_clustered(n, 50, 0xC0FF_EE12_3456_789A);
    let mut batches = Vec::new();
    for chunk_i in 0..(n / rows_per_block) {
        let chunk = &vectors[chunk_i * rows_per_block..(chunk_i + 1) * rows_per_block];
        batches.push(make_batch(
            chunk,
            (chunk_i * rows_per_block) as i64,
            (chunk_i * rows_per_block) as i64,
        ));
    }
    let manifest = write_extent(&catalog, &format, table_id, &tref, batches).await;
    build_and_register_sidecar(
        &catalog,
        &format,
        &store,
        table_id,
        &tref,
        &manifest,
        "embedding",
    )
    .await;

    // Run many seeded queries; compare ann_topk to the brute-force oracle.
    let k = 10;
    let n_queries = 50;
    let params = AnnParams::with_k(k);
    let mut qrng = Rng::new(0x1234_5678_9ABC_DEF0);
    let mut total_recall = 0.0f64;
    for _ in 0..n_queries {
        let a = &anchors[(qrng.next_u64() % anchors.len() as u64) as usize];
        let q = norm(
            a.iter()
                .map(|c| c + 0.30 * qrng.gaussian() as f32)
                .collect(),
        );

        let hits = ann_topk(
            &catalog_dyn,
            DEFAULT_TENANT,
            &format,
            &store,
            &cache,
            &tref,
            "embedding",
            &q,
            &params,
            None,
        )
        .await
        .unwrap();
        assert!(!hits.is_empty(), "expected ANN hits");
        assert!(hits.len() <= k);
        // Distances must be sorted ascending.
        for w in hits.windows(2) {
            assert!(w[0].distance <= w[1].distance + 1e-9, "hits not sorted");
        }

        let got: std::collections::HashSet<usize> = hits
            .iter()
            .map(|h| hit_to_flat_idx(h.addr.block.0, h.addr.row, rows_per_block))
            .collect();
        let want: std::collections::HashSet<usize> =
            exact_topk(&q, &vectors, k).into_iter().collect();
        let hit = got.intersection(&want).count();
        total_recall += hit as f64 / k as f64;
    }
    let recall = total_recall / n_queries as f64;
    eprintln!("ann_topk recall@{k} (with sidecar) = {recall:.4} over {n_queries} queries");
    assert!(
        recall >= 0.95,
        "recall@{k} = {recall:.4} below 0.95 threshold"
    );

    let _ = std::fs::remove_dir_all(&cache_root);
}

#[tokio::test]
async fn ann_topk_falls_back_to_exact_without_sidecar() {
    let (catalog, format, store, cache, cache_root) = setup().await;
    let catalog_dyn: Arc<dyn Catalog> = Arc::new(catalog.clone());

    let db_id = catalog.create_database("default").await.unwrap();
    let table_id = catalog
        .create_table(db_id, "vecs", schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "vecs").await.unwrap();

    // Smaller corpus, NO sidecar registered → must exact-scan and be perfect.
    let n = 800;
    let rows_per_block = 1_000;
    let (vectors, anchors) = gen_clustered(n, 20, 0x0BAD_F00D_1234_5678);
    let batch = make_batch(&vectors, 0, 0);
    let _manifest = write_extent(&catalog, &format, table_id, &tref, vec![batch]).await;
    // (no build_and_register_sidecar — this is the fallback case)

    let k = 10;
    let n_queries = 25;
    let params = AnnParams::with_k(k);
    let mut qrng = Rng::new(0xFEED_FACE_CAFE_BEEF);
    let mut total_recall = 0.0f64;
    for _ in 0..n_queries {
        let a = &anchors[(qrng.next_u64() % anchors.len() as u64) as usize];
        let q = norm(
            a.iter()
                .map(|c| c + 0.30 * qrng.gaussian() as f32)
                .collect(),
        );

        let hits = ann_topk(
            &catalog_dyn,
            DEFAULT_TENANT,
            &format,
            &store,
            &cache,
            &tref,
            "embedding",
            &q,
            &params,
            None,
        )
        .await
        .unwrap();
        assert_eq!(hits.len(), k, "fallback must return exactly k hits");

        let got: std::collections::HashSet<usize> = hits
            .iter()
            .map(|h| hit_to_flat_idx(h.addr.block.0, h.addr.row, rows_per_block))
            .collect();
        let want: std::collections::HashSet<usize> =
            exact_topk(&q, &vectors, k).into_iter().collect();
        let hit = got.intersection(&want).count();
        total_recall += hit as f64 / k as f64;
    }
    let recall = total_recall / n_queries as f64;
    eprintln!("ann_topk recall@{k} (fallback, no sidecar) = {recall:.4}");
    // Exact fallback must be perfect modulo distance ties on the unit sphere.
    assert!(
        recall >= 0.99,
        "fallback recall@{k} = {recall:.4} should be ~1.0"
    );

    let _ = std::fs::remove_dir_all(&cache_root);
}
