//! Integration test for the async `embed_backfill` executor.
//!
//! Exercises the real pipeline end-to-end against real components — no Docker:
//! a SQLite catalog (`set_table_embed_config`, `get/put_cached_embeddings`,
//! `list_extents`, snapshot add/remove), an `InMemory` object store, the
//! telemetry segment format (`open_extent` / `read_block` / `start_extent`),
//! and a FAKE deterministic [`EmbeddingBackend`].
//!
//! Flow:
//! 1. write a table `(id, msg: Utf8, embedding: FixedSizeList<Float32, N> all
//!    NULL)` with one extent; declare a `table_embed_config(msg → embedding)`;
//! 2. run the executor → assert the resulting LIVE extent has non-NULL
//!    embeddings equal to the fake backend's output per `msg`, and the cache
//!    now holds N distinct entries;
//! 3. re-run → cache hit: the fake backend's `embed()` call-count does NOT
//!    increase, and the result is unchanged (idempotent: already-embedded
//!    extent is skipped before any embed call).
//!
//! SQLite + `InMemory` is deliberate: deterministic, no Docker, and validates
//! the local/SQLite-mode contract — the executor holds no PG-specific
//! assumptions (it talks only to the `Catalog`, `SegmentFormat`, and
//! `EmbeddingBackend` traits).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use arrow_array::builder::{FixedSizeListBuilder, Float32Builder};
use arrow_array::cast::AsArray;
use arrow_array::types::Float32Type;
use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::{
    Catalog, ExtentManifest, PrunePredicate, SnapshotSummary, TableConfig, TableEmbedConfig,
    TableRef,
};
use kyma_core::crypto::content_hash_hex;
use kyma_core::fabric::{ClaimedJob, JOB_EMBED_BACKFILL};
use kyma_core::segment_format::{BlockPredicate, OpenExtentInput, SegmentFormat};
use kyma_core::tenant::DEFAULT_TENANT;
use kyma_core::types::{SchemaRef, TableId};
use kyma_embed::{EmbedError, EmbeddingBackend};
use kyma_format_tlm::TelemetryFormat;
use kyma_jobs::embed_backfill::EmbedBackfillExecutor;
use kyma_jobs::{JobCtx, JobExecutor, JobQueue, ProgressSink};
use object_store::memory::InMemory;
use object_store::ObjectStore;
use serde_json::{json, Value as Json};
use uuid::Uuid;

const DIM: usize = 8;
const MODEL_ID: &str = "fake/hash-unit-v1";

// ---------------------------------------------------------------------------
// Fake deterministic embedding backend
// ---------------------------------------------------------------------------

/// Deterministic: maps each text to a DIM-length unit vector derived from its
/// sha256, so the same text always embeds to the same vector and we can assert
/// exact equality. Counts `embed()` invocations (call granularity) and the
/// total number of texts embedded.
#[derive(Debug)]
struct FakeEmbedder {
    calls: AtomicUsize,
    texts_embedded: AtomicUsize,
}

impl FakeEmbedder {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            texts_embedded: AtomicUsize::new(0),
        }
    }
}

/// The vector the fake backend produces for `text` — also used by the oracle.
fn fake_vector(text: &str) -> Vec<f32> {
    let h = content_hash_hex(text.as_bytes());
    let bytes = h.as_bytes();
    let mut v: Vec<f32> = (0..DIM)
        .map(|i| {
            // Derive a stable f32 per dimension from the hash hex.
            let b = bytes[(i * 3) % bytes.len()] as f32;
            (b - 64.0) / 64.0
        })
        .collect();
    let norm = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if norm > 1e-9 {
        for x in &mut v {
            *x /= norm;
        }
    } else {
        v[0] = 1.0;
    }
    v
}

#[async_trait]
impl EmbeddingBackend for FakeEmbedder {
    fn id(&self) -> &str {
        MODEL_ID
    }
    fn dimension(&self) -> u16 {
        DIM as u16
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.texts_embedded.fetch_add(texts.len(), Ordering::SeqCst);
        Ok(texts.iter().map(|t| fake_vector(t)).collect())
    }
}

// ---------------------------------------------------------------------------
// Schema + extent helpers
// ---------------------------------------------------------------------------

fn schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("msg", DataType::Utf8, true),
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

/// Build a batch with all-NULL embedding slots (the ingest-side "land without
/// embedding" shape).
fn make_unembedded_batch(rows: &[(i64, &str)]) -> RecordBatch {
    let ids = Int64Array::from(rows.iter().map(|(i, _)| *i).collect::<Vec<_>>());
    let msgs = StringArray::from(rows.iter().map(|(_, m)| *m).collect::<Vec<_>>());
    // All-null FixedSizeList embedding column.
    let values = Float32Builder::new();
    let mut b = FixedSizeListBuilder::new(values, DIM as i32).with_field(Arc::new(Field::new(
        "item",
        DataType::Float32,
        true,
    )));
    for _ in rows {
        for _ in 0..DIM {
            b.values().append_null();
        }
        b.append(false);
    }
    let emb = b.finish();
    RecordBatch::try_new(schema(), vec![Arc::new(ids), Arc::new(msgs), Arc::new(emb)]).unwrap()
}

async fn write_extent(
    catalog: &SqliteCatalog,
    format: &Arc<dyn SegmentFormat>,
    table_id: TableId,
    tref: &TableRef,
    batch: RecordBatch,
) -> ExtentManifest {
    let mut writer = format.start_extent(schema(), 0).await.unwrap();
    let rows = batch.num_rows() as u64;
    writer.append(batch).await.unwrap();
    let res = writer.finish().await.unwrap();
    let manifest = ExtentManifest {
        id: res.extent_id,
        table_id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: res.object_path.clone(),
        byte_size: res.byte_size,
        row_count: res.row_count,
        min_timestamp: None,
        max_timestamp: None,
        column_stats: res.column_stats.clone(),
        present_paths: res.present_paths.clone(),
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    };
    let mut txn = catalog.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(manifest.clone()).await.unwrap();
    txn.commit(SnapshotSummary {
        rows_added: rows as i64,
        operation: "ingest".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    manifest
}

/// Read every live extent's rows back as `(msg, Option<embedding vec>)`.
async fn read_live_rows(
    catalog: &Arc<dyn Catalog>,
    format: &Arc<dyn SegmentFormat>,
    table_id: TableId,
) -> Vec<(String, Option<Vec<f32>>)> {
    let tref = catalog.lookup_table("default", "msgs").await.unwrap();
    let extents = catalog
        .list_extents_in_tenant(
            DEFAULT_TENANT,
            table_id,
            tref.current_snapshot_id,
            &PrunePredicate::default(),
        )
        .await
        .unwrap();
    let mut out = Vec::new();
    for m in extents {
        let reader = format
            .open_extent(OpenExtentInput {
                extent_id: m.id,
                table_id,
                schema: tref.schema.clone(),
                object_path: m.object_path.clone(),
                byte_size: m.byte_size,
            })
            .await
            .unwrap();
        for bid in reader.pruned_blocks(&BlockPredicate::All).await.unwrap() {
            let b = reader.read_block(bid, &[]).await.unwrap();
            let msgs = b.column_by_name("msg").unwrap().as_string::<i32>().clone();
            let emb = b
                .column_by_name("embedding")
                .unwrap()
                .as_fixed_size_list()
                .clone();
            for r in 0..b.num_rows() {
                let msg = msgs.value(r).to_string();
                let vec = if emb.is_null(r) {
                    None
                } else {
                    let prim = emb.value(r);
                    let p = prim.as_primitive::<Float32Type>();
                    Some(p.values().to_vec())
                };
                out.push((msg, vec));
            }
        }
    }
    out
}

fn ctx_and_job(payload: Json) -> (JobCtx, ClaimedJob) {
    let queue: Arc<dyn JobQueue> = Arc::new(StubQueue);
    let job = ClaimedJob {
        id: Uuid::new_v4(),
        tenant_id: DEFAULT_TENANT.as_uuid(),
        kind: JOB_EMBED_BACKFILL.to_string(),
        payload,
        attempt: 1,
        max_attempts: 3,
        claim_expires_at: chrono::Utc::now() + chrono::Duration::seconds(60),
        credentials: None,
    };
    let ctx = JobCtx {
        queue: queue.clone(),
        progress: ProgressSink::new(queue, job.id),
    };
    (ctx, job)
}

struct StubQueue;
#[async_trait]
impl JobQueue for StubQueue {
    async fn claim(&self, _kinds: &[String], _max: usize) -> anyhow::Result<Vec<ClaimedJob>> {
        Ok(Vec::new())
    }
    async fn progress(&self, _job_id: Uuid, _snapshot: Json) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn complete(&self, _job_id: Uuid, _result: Json) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn fail(&self, _job_id: Uuid, _error: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn fail_terminal(&self, _job_id: Uuid, _error: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn extend_lease(&self, _job_id: Uuid, _lease_secs: i64) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn link_dreaming_run(&self, _job_id: Uuid, _run_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn embed_backfill_fills_embeddings_and_caches() {
    let catalog = SqliteCatalog::connect_in_memory().await.unwrap();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "kyma"));

    let db = catalog.create_database("default").await.unwrap();
    let table_id = catalog
        .create_table(db, "msgs", schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "msgs").await.unwrap();

    // One extent with NULL embeddings. "beta" appears twice → duplicate text
    // must be embedded only once (cache + dedup within the job).
    let rows = vec![
        (1i64, "alpha"),
        (2, "beta"),
        (3, "gamma"),
        (4, "beta"),
        (5, ""), // empty text → must stay NULL (never embedded)
    ];
    let _manifest = write_extent(
        &catalog,
        &format,
        table_id,
        &tref,
        make_unembedded_batch(&rows),
    )
    .await;

    // Declare the per-table embed config (msg → embedding, fake model).
    let cfg = TableEmbedConfig {
        table_id,
        source_column: "msg".into(),
        embedding_column: "embedding".into(),
        model_id: MODEL_ID.into(),
        dim: DIM as u16,
        auto_embed: true,
    };
    catalog
        .set_table_embed_config(DEFAULT_TENANT, &cfg)
        .await
        .unwrap();
    // Config round-trips.
    let got = catalog
        .list_table_embed_configs(DEFAULT_TENANT, table_id)
        .await
        .unwrap();
    assert_eq!(got, vec![cfg.clone()]);

    let catalog_dyn: Arc<dyn Catalog> = Arc::new(catalog.clone());
    let embedder = Arc::new(FakeEmbedder::new());
    let exec = EmbedBackfillExecutor::new(catalog_dyn.clone(), format.clone(), embedder.clone());

    // Gather the live extent ids to address in the payload.
    let live = catalog
        .list_extents_in_tenant(
            DEFAULT_TENANT,
            table_id,
            tref.current_snapshot_id,
            &PrunePredicate::default(),
        )
        .await
        .unwrap();
    let extent_ids: Vec<Uuid> = live.iter().map(|m| *m.id.as_uuid()).collect();
    assert_eq!(extent_ids.len(), 1);

    let payload = json!({
        "table_id": table_id.as_uuid(),
        "extent_ids": extent_ids,
        "source_column": "msg",
        "embedding_column": "embedding",
        "model_id": MODEL_ID,
    });

    // ---- First run: fills embeddings ----
    let (ctx, job) = ctx_and_job(payload.clone());
    let result = exec.run(&ctx, &job).await.unwrap();
    assert_eq!(result, json!({ "embedded": 1, "skipped": 0, "failed": 0 }));

    // Each row's embedding matches the fake backend (empty text stays NULL).
    let live_rows = read_live_rows(&catalog_dyn, &format, table_id).await;
    assert_eq!(live_rows.len(), 5, "row count preserved");
    for (msg, vec) in &live_rows {
        if msg.is_empty() {
            assert!(vec.is_none(), "empty text must stay NULL");
        } else {
            let v = vec
                .as_ref()
                .unwrap_or_else(|| panic!("'{msg}' should be embedded"));
            assert_eq!(
                v,
                &fake_vector(msg),
                "embedding for '{msg}' matches fake backend"
            );
        }
    }

    // The cache now holds one entry per DISTINCT non-empty text: alpha, beta,
    // gamma = 3. "beta" (twice) embedded once; "" never embedded.
    let distinct: std::collections::HashSet<&str> = rows
        .iter()
        .map(|(_, m)| *m)
        .filter(|m| !m.is_empty())
        .collect();
    let hashes: Vec<String> = distinct
        .iter()
        .map(|t| content_hash_hex(t.as_bytes()))
        .collect();
    let cached = catalog
        .get_cached_embeddings(DEFAULT_TENANT, MODEL_ID, &hashes)
        .await
        .unwrap();
    assert_eq!(
        cached.len(),
        distinct.len(),
        "cache has one entry per distinct text"
    );
    for t in &distinct {
        let h = content_hash_hex(t.as_bytes());
        assert_eq!(cached.get(&h).unwrap(), &fake_vector(t));
    }
    // The fake backend was asked to embed exactly the 3 distinct misses (one
    // embed() call with 3 texts).
    assert_eq!(embedder.texts_embedded.load(Ordering::SeqCst), 3);
    let calls_after_first = embedder.calls.load(Ordering::SeqCst);
    assert_eq!(calls_after_first, 1);

    // ---- Second run on the now-embedded extent: idempotent skip, no embed ----
    // Re-resolve the (now superseded) live extent id.
    let tref2 = catalog.lookup_table("default", "msgs").await.unwrap();
    let live2 = catalog
        .list_extents_in_tenant(
            DEFAULT_TENANT,
            table_id,
            tref2.current_snapshot_id,
            &PrunePredicate::default(),
        )
        .await
        .unwrap();
    let extent_ids2: Vec<Uuid> = live2.iter().map(|m| *m.id.as_uuid()).collect();
    assert_eq!(
        extent_ids2.len(),
        1,
        "old extent superseded by exactly one new extent"
    );
    assert_ne!(
        extent_ids2, extent_ids,
        "new extent id differs (rewrite, not mutate)"
    );

    let payload2 = json!({
        "table_id": table_id.as_uuid(),
        "extent_ids": extent_ids2,
        "source_column": "msg",
        "embedding_column": "embedding",
        "model_id": MODEL_ID,
    });
    let (ctx2, job2) = ctx_and_job(payload2);
    let result2 = exec.run(&ctx2, &job2).await.unwrap();
    assert_eq!(
        result2,
        json!({ "embedded": 0, "skipped": 1, "failed": 0 }),
        "already-embedded extent is skipped"
    );
    // The fake backend was NOT invoked again (no new embed() call).
    assert_eq!(
        embedder.calls.load(Ordering::SeqCst),
        calls_after_first,
        "no re-embedding on the already-filled extent"
    );
    assert_eq!(embedder.texts_embedded.load(Ordering::SeqCst), 3);

    // Data unchanged after the no-op re-run.
    let live_rows2 = read_live_rows(&catalog_dyn, &format, table_id).await;
    assert_eq!(live_rows2, live_rows, "re-run leaves data unchanged");
}

#[tokio::test]
async fn embed_backfill_cache_hit_skips_backend_across_extents() {
    // A second extent with text that was ALREADY embedded (and cached) by a
    // prior extent must hit the cache: the backend is not called for those
    // texts. Validates the content-hash cache across extents/jobs.
    let catalog = SqliteCatalog::connect_in_memory().await.unwrap();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "kyma"));

    let db = catalog.create_database("default").await.unwrap();
    let table_id = catalog
        .create_table(db, "msgs", schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "msgs").await.unwrap();

    // Pre-seed the cache with "alpha" and "beta" as if a prior job embedded
    // them, so this job's only miss is "delta".
    for t in ["alpha", "beta"] {
        catalog
            .put_cached_embeddings(
                DEFAULT_TENANT,
                MODEL_ID,
                DIM as u16,
                &[(content_hash_hex(t.as_bytes()), fake_vector(t))],
            )
            .await
            .unwrap();
    }

    let m = write_extent(
        &catalog,
        &format,
        table_id,
        &tref,
        make_unembedded_batch(&[(1, "alpha"), (2, "beta"), (3, "delta")]),
    )
    .await;

    let catalog_dyn: Arc<dyn Catalog> = Arc::new(catalog.clone());
    let embedder = Arc::new(FakeEmbedder::new());
    let exec = EmbedBackfillExecutor::new(catalog_dyn.clone(), format.clone(), embedder.clone());

    let payload = json!({
        "table_id": table_id.as_uuid(),
        "extent_ids": [m.id.as_uuid()],
        "source_column": "msg",
        "embedding_column": "embedding",
        "model_id": MODEL_ID,
    });
    let (ctx, job) = ctx_and_job(payload);
    let result = exec.run(&ctx, &job).await.unwrap();
    assert_eq!(result, json!({ "embedded": 1, "skipped": 0, "failed": 0 }));

    // Only "delta" was a miss → exactly 1 text embedded (alpha/beta from cache).
    assert_eq!(
        embedder.texts_embedded.load(Ordering::SeqCst),
        1,
        "cache hit for alpha/beta, only delta embedded"
    );

    // All three rows are correctly filled regardless of cache vs fresh.
    let live_rows = read_live_rows(&catalog_dyn, &format, table_id).await;
    for (msg, vec) in &live_rows {
        assert_eq!(vec.as_ref().unwrap(), &fake_vector(msg));
    }
}

#[tokio::test]
async fn embed_backfill_rejects_model_id_mismatch() {
    // Safety: the executor must refuse when the injected backend's id differs
    // from the payload model_id — never writing mismatched-model vectors.
    let catalog = SqliteCatalog::connect_in_memory().await.unwrap();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store.clone(), "kyma"));
    let db = catalog.create_database("default").await.unwrap();
    let table_id = catalog
        .create_table(db, "msgs", schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "msgs").await.unwrap();
    let m = write_extent(
        &catalog,
        &format,
        table_id,
        &tref,
        make_unembedded_batch(&[(1, "alpha")]),
    )
    .await;

    let catalog_dyn: Arc<dyn Catalog> = Arc::new(catalog.clone());
    let exec = EmbedBackfillExecutor::new(catalog_dyn, format, Arc::new(FakeEmbedder::new()));
    let (ctx, job) = ctx_and_job(json!({
        "table_id": table_id.as_uuid(),
        "extent_ids": [m.id.as_uuid()],
        "source_column": "msg",
        "embedding_column": "embedding",
        "model_id": "some/other-model",
    }));
    let err = exec.run(&ctx, &job).await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("model_id") || msg.contains("mismatched"),
        "expected model-id mismatch error, got: {msg}"
    );
}
