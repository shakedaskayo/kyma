//! S2.2 committer: stage extents → drain → exactly-once commit (testcontainers PG).
//!
//! Stages 3 extents (2 for table A, 1 for table B) into `staged_extents`, runs
//! one committer tick, and asserts: both tables' snapshots advanced and their
//! extents are live, the staged table is empty, and re-staging a batch_id is an
//! idempotent no-op.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use pensieve_catalog::PostgresCatalog;
use pensieve_core::catalog::{Catalog, ExtentManifest, PrunePredicate, TableConfig, TableRef};
use pensieve_core::tenant::DEFAULT_TENANT;
use pensieve_core::types::{ExtentId, TableId};
use pensieve_ingest_core::committer::Committer;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn connect() -> (PostgresCatalog, testcontainers::ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_user("pensieve")
        .with_password("pensieve_dev")
        .with_db_name("pensieve")
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://pensieve:pensieve_dev@localhost:{port}/pensieve");
    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("connect + migrate");
    (catalog, container)
}

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new("msg", DataType::Utf8, true)]))
}

fn manifest_for(tref: &TableRef) -> ExtentManifest {
    ExtentManifest {
        id: ExtentId::new(),
        table_id: tref.id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: format!("staged/{}.pnsv", Uuid::new_v4()),
        byte_size: 2048,
        row_count: 100,
        min_timestamp: None,
        max_timestamp: None,
        column_stats: serde_json::json!({}),
        present_paths: vec![],
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    }
}

async fn live_extent_count(catalog: &PostgresCatalog, table: &str) -> usize {
    let tref = catalog.lookup_table("default", table).await.unwrap();
    catalog
        .list_extents_in_tenant(
            DEFAULT_TENANT,
            tref.id,
            tref.current_snapshot_id,
            &PrunePredicate::default(),
        )
        .await
        .unwrap()
        .len()
}

#[tokio::test]
async fn stage_then_commit_is_exactly_once() {
    let (catalog, _c) = connect().await;
    let db = catalog.create_database("default").await.unwrap();
    catalog
        .create_table(db, "a", schema(), TableConfig::default())
        .await
        .unwrap();
    catalog
        .create_table(db, "b", schema(), TableConfig::default())
        .await
        .unwrap();
    let a = catalog.lookup_table("default", "a").await.unwrap();
    let b = catalog.lookup_table("default", "b").await.unwrap();

    // Stage 2 extents for A, 1 for B.
    for _ in 0..2 {
        let m = manifest_for(&a);
        assert!(catalog
            .stage_extent(DEFAULT_TENANT, Uuid::new_v4(), &m)
            .await
            .unwrap());
    }
    let m_b = manifest_for(&b);
    let b_batch = Uuid::new_v4();
    assert!(catalog
        .stage_extent(DEFAULT_TENANT, b_batch, &m_b)
        .await
        .unwrap());

    // Re-staging the same batch_id is an idempotent no-op.
    assert!(!catalog
        .stage_extent(DEFAULT_TENANT, b_batch, &m_b)
        .await
        .unwrap());

    let staged = catalog
        .list_staged_extents(DEFAULT_TENANT, 1000)
        .await
        .unwrap();
    assert_eq!(staged.len(), 3, "3 staged, dup ignored");
    // Nothing committed yet.
    assert_eq!(live_extent_count(&catalog, "a").await, 0);
    assert_eq!(live_extent_count(&catalog, "b").await, 0);

    // One committer tick drains + commits both tables' groups.
    let committer = Committer::new(Arc::new(catalog.clone()), DEFAULT_TENANT);
    let n = committer.tick().await.unwrap();
    assert_eq!(n, 3, "committed all 3 staged extents");

    // Staged table emptied; extents now live under each table's new snapshot.
    assert_eq!(
        catalog
            .list_staged_extents(DEFAULT_TENANT, 1000)
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        live_extent_count(&catalog, "a").await,
        2,
        "table A snapshot has 2 extents"
    );
    assert_eq!(
        live_extent_count(&catalog, "b").await,
        1,
        "table B snapshot has 1 extent"
    );

    // A second tick with nothing staged is a no-op.
    assert_eq!(committer.tick().await.unwrap(), 0);
}

#[tokio::test]
async fn staged_write_path_acks_without_commit_then_committer_commits() {
    use arrow_array::{Int64Array, RecordBatch};
    use pensieve_core::segment_format::SegmentFormat;
    use pensieve_ingest_core::WritePath;
    use object_store::memory::InMemory;
    use object_store::ObjectStore;
    use std::sync::Arc as StdArc;

    let (catalog, _c) = connect().await;
    let db = catalog.create_database("default").await.unwrap();
    let int_schema = StdArc::new(Schema::new(vec![Field::new("id", DataType::Int64, true)]));
    catalog
        .create_table(db, "events", int_schema.clone(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "events").await.unwrap();
    let snap_before = tref.current_snapshot_id;

    // Staged WritePath over an InMemory store + TLM format.
    let store: StdArc<dyn ObjectStore> = StdArc::new(InMemory::new());
    let format: StdArc<dyn SegmentFormat> =
        StdArc::new(pensieve_format_tlm::TelemetryFormat::new(store.clone(), "pensieve"));
    let catalog_dyn: StdArc<dyn Catalog> = StdArc::new(catalog.clone());
    let wp = WritePath::new(catalog_dyn.clone(), format).with_staged_mode();

    let batch = RecordBatch::try_new(
        int_schema.clone(),
        vec![StdArc::new(Int64Array::from(vec![1, 2, 3]))],
    )
    .unwrap();
    let ack = wp.ingest("default", &tref, vec![batch]).await.unwrap();
    assert_eq!(ack.rows_ingested, 3);
    // Staged ack carries the CURRENT snapshot — NOT a new one (not committed yet).
    assert_eq!(
        ack.snapshot_id, snap_before,
        "staged ack = current snapshot"
    );

    // The extent is staged, the table snapshot is unchanged, and it is NOT yet
    // visible (read-your-writes is intentionally relaxed in staged mode).
    assert_eq!(
        catalog
            .list_staged_extents(DEFAULT_TENANT, 100)
            .await
            .unwrap()
            .len(),
        1
    );
    let tref_mid = catalog.lookup_table("default", "events").await.unwrap();
    assert_eq!(
        tref_mid.current_snapshot_id, snap_before,
        "no commit at ack time"
    );
    assert_eq!(live_extent_count(&catalog, "events").await, 0);

    // The committer commits it; now it is live and destaged.
    let committer = Committer::new(catalog_dyn, DEFAULT_TENANT);
    assert_eq!(committer.tick().await.unwrap(), 1);
    assert_eq!(
        live_extent_count(&catalog, "events").await,
        1,
        "committed + visible"
    );
    assert_eq!(
        catalog
            .list_staged_extents(DEFAULT_TENANT, 100)
            .await
            .unwrap()
            .len(),
        0
    );
}
