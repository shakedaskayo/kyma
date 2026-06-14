//! S2.2 committer: stage extents → drain → exactly-once commit (testcontainers PG).
//!
//! Stages 3 extents (2 for table A, 1 for table B) into `staged_extents`, runs
//! one committer tick, and asserts: both tables' snapshots advanced and their
//! extents are live, the staged table is empty, and re-staging a batch_id is an
//! idempotent no-op.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, ExtentManifest, PrunePredicate, TableConfig, TableRef};
use kyma_core::tenant::DEFAULT_TENANT;
use kyma_core::types::{ExtentId, TableId};
use kyma_ingest_core::committer::Committer;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn connect() -> (PostgresCatalog, testcontainers::ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_user("kyma")
        .with_password("kyma_dev")
        .with_db_name("kyma")
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");
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
        object_path: format!("staged/{}.kyma", Uuid::new_v4()),
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
