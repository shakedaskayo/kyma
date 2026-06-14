//! `AnnMaintainExecutor` end-to-end over SQLite + InMemory (no Docker, no TLM).
//!
//! The executor only reads each extent's IVF *centroids*, so we craft valid
//! KYIV sidecar blobs directly (via `kyma_index_vector::file::write`) with known
//! centroids — extent A near +e0, extent B near -e0 — upload them, register the
//! sidecar rows, then run the executor and assert: a tree row + blob are written,
//! the tree routes a +e0 query to extent A only, a re-run is idempotent
//! (`unchanged`), and adding an extent bumps the generation.

use std::sync::Arc;

use async_trait::async_trait;
use kyma_catalog_sqlite::SqliteCatalog;
use kyma_core::catalog::{Catalog, ExtentManifest, SnapshotSummary, TableConfig};
use kyma_core::fabric::{ClaimedJob, JOB_ANN_MAINTAIN};
use kyma_core::index_sidecar::{IndexSidecarDescriptor, SidecarKind};
use kyma_core::tenant::DEFAULT_TENANT;
use kyma_core::types::{ExtentId, TableId};
use kyma_index_vector::file as ivf_file;
use kyma_index_vector::global_tree::GlobalTree;
use kyma_jobs::ann_maintain::AnnMaintainExecutor;
use kyma_jobs::{JobCtx, JobExecutor, JobQueue, ProgressSink};
use object_store::memory::InMemory;
use object_store::{path::Path as ObjPath, ObjectStore};
use serde_json::{json, Value as Json};
use uuid::Uuid;

const DIM: usize = 8;
const MODEL: &str = "test/model";

fn norm(mut v: Vec<f32>) -> Vec<f32> {
    let n = (v.iter().map(|x| x * x).sum::<f32>()).sqrt();
    if n > 1e-9 {
        for x in &mut v {
            *x /= n;
        }
    }
    v
}

/// `n` centroids jittered around `sign * e0`.
fn region_centroids(n: usize, sign: f32, seed: u64) -> Vec<Vec<f32>> {
    let mut s = seed;
    let mut next = || {
        // tiny deterministic LCG jitter
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((s >> 33) as f32 / u32::MAX as f32 - 0.5) * 0.02
    };
    (0..n)
        .map(|_| {
            let mut v = vec![0.0f32; DIM];
            v[0] = sign * 5.0;
            for x in v.iter_mut() {
                *x += next();
            }
            norm(v)
        })
        .collect()
}

/// Write a KYIV IVF sidecar blob with the given centroids (empty posting lists —
/// the maintain executor only reads centroids), upload it, and register the row.
async fn add_extent_with_sidecar(
    catalog: &SqliteCatalog,
    store: &Arc<dyn ObjectStore>,
    table_id: TableId,
    schema_snapshot_id: kyma_core::types::SchemaSnapshotId,
    centroids: &[Vec<f32>],
) -> ExtentId {
    let id = ExtentId::new();
    let manifest = ExtentManifest {
        id,
        table_id,
        schema_snapshot_id,
        object_path: format!("test/extents/{}.kyma", id.as_uuid()),
        byte_size: 64,
        row_count: 100,
        min_timestamp: None,
        max_timestamp: None,
        column_stats: json!({}),
        present_paths: vec![],
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    };
    let mut txn = catalog.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(manifest).await.unwrap();
    txn.commit(SnapshotSummary {
        operation: "ingest".into(),
        ..Default::default()
    })
    .await
    .unwrap();

    // Empty partitions, one per centroid (write() requires matching lengths).
    let partitions: Vec<Vec<ivf_file::Entry>> = vec![Vec::new(); centroids.len()];
    let blob = ivf_file::write(
        DIM,
        ivf_file::METRIC_COSINE,
        centroids,
        &partitions,
        Some(MODEL),
    );
    let path = format!(
        "{DEFAULT_TENANT}/indexes/{}/embedding.ivf_rabitq",
        id.as_uuid()
    );
    store
        .put(&ObjPath::from(path.as_str()), blob.to_vec().into())
        .await
        .unwrap();
    catalog
        .register_index_sidecar(
            DEFAULT_TENANT,
            &IndexSidecarDescriptor {
                id: Uuid::new_v4(),
                extent_id: id,
                table_id,
                column: "embedding".into(),
                kind: SidecarKind::IvfRabitq,
                object_path: path,
                byte_size: blob.len() as u64,
                params: json!({ "nlist": centroids.len() }),
                embedding_model_id: Some(MODEL.into()),
                created_at: chrono::Utc::now(),
            },
        )
        .await
        .unwrap();
    id
}

struct StubQueue;
#[async_trait]
impl JobQueue for StubQueue {
    async fn claim(&self, _k: &[String], _m: usize) -> anyhow::Result<Vec<ClaimedJob>> {
        Ok(Vec::new())
    }
    async fn progress(&self, _j: Uuid, _s: Json) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn complete(&self, _j: Uuid, _r: Json) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn fail(&self, _j: Uuid, _e: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn fail_terminal(&self, _j: Uuid, _e: &str) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn extend_lease(&self, _j: Uuid, _s: i64) -> anyhow::Result<bool> {
        Ok(true)
    }
    async fn link_dreaming_run(&self, _j: Uuid, _r: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
}

fn ctx_and_job(table_id: TableId) -> (JobCtx, ClaimedJob) {
    let queue: Arc<dyn JobQueue> = Arc::new(StubQueue);
    let job = ClaimedJob {
        id: Uuid::new_v4(),
        tenant_id: DEFAULT_TENANT.as_uuid(),
        kind: JOB_ANN_MAINTAIN.to_string(),
        payload: json!({
            "table_id": table_id.as_uuid(),
            "column": "embedding",
            "embedding_model_id": MODEL,
        }),
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

#[tokio::test]
async fn builds_routes_idempotent_and_bumps_generation() {
    let cat = SqliteCatalog::connect_in_memory().await.unwrap();
    let db = cat.create_database("default").await.unwrap();
    let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "embedding",
        arrow_schema::DataType::FixedSizeList(
            Arc::new(arrow_schema::Field::new(
                "item",
                arrow_schema::DataType::Float32,
                true,
            )),
            DIM as i32,
        ),
        true,
    )]));
    let table_id = cat
        .create_table(db, "docs", schema, TableConfig::default())
        .await
        .unwrap();
    let tref = cat.lookup_table("default", "docs").await.unwrap();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());

    // Extent A near +e0, extent B near -e0.
    let ext_a = add_extent_with_sidecar(
        &cat,
        &store,
        table_id,
        tref.schema_snapshot_id,
        &region_centroids(4, 1.0, 1),
    )
    .await;
    let _ext_b = add_extent_with_sidecar(
        &cat,
        &store,
        table_id,
        tref.schema_snapshot_id,
        &region_centroids(4, -1.0, 2),
    )
    .await;

    let catalog: Arc<dyn Catalog> = Arc::new(cat.clone());
    let exec = AnnMaintainExecutor::new(catalog.clone(), store.clone());

    // Build.
    let (ctx, job) = ctx_and_job(table_id);
    let res = exec.run(&ctx, &job).await.unwrap();
    assert_eq!(res["status"], "built");
    assert_eq!(res["generation"], 1);
    assert_eq!(res["source_extents"], 2);

    // Tree row + blob exist; the blob parses and routes a +e0 query to extent A.
    let row = catalog
        .get_ann_tree(DEFAULT_TENANT, table_id, "embedding", Some(MODEL))
        .await
        .unwrap()
        .expect("tree row");
    let blob = store
        .get(&ObjPath::from(row.object_path.as_str()))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    let tree = GlobalTree::read(&blob).unwrap();
    assert_eq!(tree.total_refs(), 8, "all 8 extent centroids routed");

    let q = norm({
        let mut v = vec![0.0f32; DIM];
        v[0] = 5.0;
        v
    });
    let sel = tree.select_partitions(&q, 2);
    assert!(!sel.is_empty());
    assert!(
        sel.iter().all(|r| r.extent == *ext_a.as_uuid()),
        "a +e0 query must route only to extent A (the +e0 extent)"
    );

    // Idempotent re-run: same extent set → unchanged.
    let (ctx2, job2) = ctx_and_job(table_id);
    let res2 = exec.run(&ctx2, &job2).await.unwrap();
    assert_eq!(res2["status"], "unchanged");
    assert_eq!(res2["generation"], 1);

    // Add a third extent → fingerprint changes → rebuild bumps generation.
    let _ext_c = add_extent_with_sidecar(
        &cat,
        &store,
        table_id,
        tref.schema_snapshot_id,
        &region_centroids(4, 1.0, 3),
    )
    .await;
    let (ctx3, job3) = ctx_and_job(table_id);
    let res3 = exec.run(&ctx3, &job3).await.unwrap();
    assert_eq!(res3["status"], "built");
    assert_eq!(res3["generation"], 2, "new extent set → generation bumps");
    assert_eq!(res3["source_extents"], 3);
}

#[tokio::test]
async fn no_sidecars_returns_no_sidecars() {
    let cat = SqliteCatalog::connect_in_memory().await.unwrap();
    let db = cat.create_database("default").await.unwrap();
    let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
        "embedding",
        arrow_schema::DataType::FixedSizeList(
            Arc::new(arrow_schema::Field::new(
                "item",
                arrow_schema::DataType::Float32,
                true,
            )),
            DIM as i32,
        ),
        true,
    )]));
    let table_id = cat
        .create_table(db, "docs", schema, TableConfig::default())
        .await
        .unwrap();
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
    let catalog: Arc<dyn Catalog> = Arc::new(cat.clone());
    let exec = AnnMaintainExecutor::new(catalog.clone(), store);

    let (ctx, job) = ctx_and_job(table_id);
    let res = exec.run(&ctx, &job).await.unwrap();
    assert_eq!(res["status"], "no_sidecars");
    assert!(catalog
        .get_ann_tree(DEFAULT_TENANT, table_id, "embedding", Some(MODEL))
        .await
        .unwrap()
        .is_none());
}
