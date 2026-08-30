//! `IndexBuildExecutor` plumbing test — no Docker: in-memory `SQLite` catalog,
//! in-memory object store, a stub `SegmentFormat`, and a no-op test
//! `SidecarBuilder`. Verifies the S0.4 flow: build → upload → register, the
//! idempotent re-run (skip already-indexed extents), and the clear terminal
//! failure when no builder is registered for the requested kind.

use arrow_array::{RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use bytes::Bytes;
use futures::StreamExt;
use pensieve_catalog_sqlite::SqliteCatalog;
use pensieve_core::catalog::{Catalog, ExtentManifest, SnapshotSummary, TableConfig};
use pensieve_core::errors::Result as PensieveResult;
use pensieve_core::fabric::{ClaimedJob, JOB_INDEX_BUILD};
use pensieve_core::index_sidecar::{BatchStream, BuiltSidecar, SidecarBuilder, SidecarKind};
use pensieve_core::segment_format::{
    BlockId, BlockPredicate, ColumnId, ExtentMetadata, ExtentReader, ExtentWriter,
    OpenExtentInput, SegmentFormat,
};
use pensieve_core::tenant::DEFAULT_TENANT;
use pensieve_core::types::SchemaRef;
use pensieve_jobs::index_build::IndexBuildExecutor;
use pensieve_jobs::{JobCtx, JobError, JobExecutor, JobQueue, ProgressSink};
use object_store::memory::InMemory;
use object_store::ObjectStore;
use serde_json::{json, Value as Json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Stubs
// ---------------------------------------------------------------------------

fn sample_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![Field::new("msg", DataType::Utf8, true)]))
}

fn sample_batch(rows: &[&str]) -> RecordBatch {
    RecordBatch::try_new(
        sample_schema(),
        vec![Arc::new(StringArray::from(rows.to_vec()))],
    )
    .unwrap()
}

/// `SegmentFormat` stub: every extent "contains" two fixed blocks.
struct StubFormat;

struct StubReader {
    meta: ExtentMetadata,
}

#[async_trait]
impl ExtentReader for StubReader {
    fn metadata(&self) -> &ExtentMetadata {
        &self.meta
    }
    async fn pruned_blocks(&self, _predicate: &BlockPredicate) -> PensieveResult<Vec<BlockId>> {
        Ok(vec![BlockId(0), BlockId(1)])
    }
    async fn read_block(&self, block: BlockId, _projection: &[ColumnId]) -> PensieveResult<RecordBatch> {
        Ok(match block {
            BlockId(0) => sample_batch(&["alpha", "beta"]),
            _ => sample_batch(&["gamma"]),
        })
    }
}

#[async_trait]
impl SegmentFormat for StubFormat {
    fn name(&self) -> &'static str {
        "stub"
    }
    fn magic(&self) -> &'static [u8] {
        b"STUB"
    }
    fn current_version(&self) -> u32 {
        1
    }
    fn max_readable_version(&self) -> u32 {
        1
    }
    async fn open_extent(&self, input: OpenExtentInput) -> PensieveResult<Arc<dyn ExtentReader>> {
        Ok(Arc::new(StubReader {
            meta: ExtentMetadata {
                row_count: 3,
                block_count: 2,
                byte_size: input.byte_size,
                schema: input.schema,
                format_version: 1,
            },
        }))
    }
    async fn start_extent(
        &self,
        _schema: SchemaRef,
        _target_bytes: u64,
    ) -> PensieveResult<Box<dyn ExtentWriter>> {
        unimplemented!("stub format is read-only")
    }
}

/// No-op builder: records how many rows it streamed; emits a tiny blob.
struct NoopBuilder {
    builds: AtomicUsize,
}

#[async_trait]
impl SidecarBuilder for NoopBuilder {
    fn kind(&self) -> SidecarKind {
        SidecarKind::TantivyFts
    }
    async fn build(
        &self,
        _extent: &ExtentManifest,
        column: &str,
        mut batches: BatchStream<'_>,
    ) -> PensieveResult<BuiltSidecar> {
        let mut rows = 0usize;
        while let Some(batch) = batches.next().await {
            rows += batch?.num_rows();
        }
        self.builds.fetch_add(1, Ordering::SeqCst);
        Ok(BuiltSidecar {
            bytes: Bytes::from(format!("noop:{column}:{rows}")),
            params: json!({ "rows": rows }),
            embedding_model_id: None,
        })
    }
}

/// `JobQueue` stub — the executor only uses it for best-effort progress.
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

fn ctx_and_job(payload: Json) -> (JobCtx, ClaimedJob) {
    let queue: Arc<dyn JobQueue> = Arc::new(StubQueue);
    let job = ClaimedJob {
        id: Uuid::new_v4(),
        tenant_id: DEFAULT_TENANT.as_uuid(),
        kind: JOB_INDEX_BUILD.to_string(),
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

/// In-memory catalog with one table and two committed extents.
async fn catalog_with_extents() -> (Arc<dyn Catalog>, pensieve_core::types::TableId, Vec<Uuid>) {
    let cat = SqliteCatalog::connect_in_memory().await.unwrap();
    let db = cat.create_database("default").await.unwrap();
    let table_id = cat
        .create_table(db, "logs", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = cat.lookup_table("default", "logs").await.unwrap();

    let mut extent_ids = Vec::new();
    for _ in 0..2 {
        let m = ExtentManifest {
            id: pensieve_core::types::ExtentId::new(),
            table_id,
            schema_snapshot_id: tref.schema_snapshot_id,
            object_path: format!("test/extents/{}.stub", Uuid::new_v4()),
            byte_size: 64,
            row_count: 3,
            min_timestamp: None,
            max_timestamp: None,
            column_stats: json!({}),
            present_paths: vec![],
            compaction_gen: 0,
            created_at: chrono::Utc::now(),
        };
        extent_ids.push(*m.id.as_uuid());
        let mut txn = cat.begin_snapshot(table_id).await.unwrap();
        txn.add_extent(m).await.unwrap();
        txn.commit(SnapshotSummary {
            operation: "ingest".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    }
    (Arc::new(cat), table_id, extent_ids)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn builds_uploads_registers_and_reruns_idempotently() {
    let (catalog, table_id, extent_ids) = catalog_with_extents().await;
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let builder = Arc::new(NoopBuilder {
        builds: AtomicUsize::new(0),
    });
    let mut builders: HashMap<SidecarKind, Arc<dyn SidecarBuilder>> = HashMap::new();
    builders.insert(SidecarKind::TantivyFts, builder.clone());
    let exec = IndexBuildExecutor::new(
        catalog.clone(),
        Arc::new(StubFormat),
        store.clone(),
        builders,
    );

    let payload = json!({
        "table_id": table_id.as_uuid(),
        "extent_ids": extent_ids,
        "column": "msg",
        "kind": "tantivy_fts",
        "params": {},
    });
    let (ctx, job) = ctx_and_job(payload.clone());
    let result = exec.run(&ctx, &job).await.unwrap();
    assert_eq!(result, json!({ "built": 2, "skipped": 0 }));
    assert_eq!(builder.builds.load(Ordering::SeqCst), 2);

    // Objects landed at the documented path convention, with builder bytes.
    let typed_ids: Vec<pensieve_core::types::ExtentId> = extent_ids
        .iter()
        .map(|u| pensieve_core::types::ExtentId::from_uuid(*u))
        .collect();
    for eid in &typed_ids {
        let path = IndexBuildExecutor::object_path(
            DEFAULT_TENANT,
            *eid,
            "msg",
            SidecarKind::TantivyFts,
        );
        let blob = store
            .get(&object_store::path::Path::from(path.as_str()))
            .await
            .expect("sidecar object uploaded")
            .bytes()
            .await
            .unwrap();
        assert_eq!(blob, Bytes::from("noop:msg:3"), "all 3 rows streamed");
    }

    // Descriptors registered with the builder's params.
    let descs = catalog
        .list_index_sidecars(DEFAULT_TENANT, table_id, &typed_ids, Some(SidecarKind::TantivyFts))
        .await
        .unwrap();
    assert_eq!(descs.len(), 2);
    for d in &descs {
        assert_eq!(d.column, "msg");
        assert_eq!(d.params, json!({ "rows": 3 }));
        assert_eq!(d.byte_size, "noop:msg:3".len() as u64);
    }

    // Re-run the same job: every extent already has its sidecar → skipped,
    // and the builder is not invoked again.
    let (ctx2, job2) = ctx_and_job(payload);
    let result2 = exec.run(&ctx2, &job2).await.unwrap();
    assert_eq!(result2, json!({ "built": 0, "skipped": 2 }));
    assert_eq!(builder.builds.load(Ordering::SeqCst), 2, "idempotent re-run");
}

#[tokio::test]
async fn missing_builder_fails_with_clear_config_error() {
    let (catalog, table_id, extent_ids) = catalog_with_extents().await;
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    // Empty builder map — the S0 wiring in pensieve-bin.
    let exec = IndexBuildExecutor::new(
        catalog,
        Arc::new(StubFormat),
        store,
        HashMap::new(),
    );
    let (ctx, job) = ctx_and_job(json!({
        "table_id": table_id.as_uuid(),
        "extent_ids": extent_ids,
        "column": "msg",
        "kind": "ivf_rabitq",
        "params": {},
    }));
    let err = exec.run(&ctx, &job).await.unwrap_err();
    match err {
        JobError::Config(msg) => {
            assert!(
                msg.contains("no sidecar builder registered") && msg.contains("ivf_rabitq"),
                "error must name the missing kind: {msg}"
            );
        }
        other => panic!("expected Config error, got {other}"),
    }
}
