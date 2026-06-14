//! IndexScheduler end-to-end: a vector column with un-indexed extents gets
//! `index_build` jobs enqueued; once a sidecar exists the extent is skipped;
//! a table with no vector column produces nothing.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use kyma_catalog::PostgresCatalog;
use kyma_compaction::IndexScheduler;
use kyma_core::catalog::{Catalog, ExtentManifest, SnapshotSummary, TableConfig};
use kyma_core::index_sidecar::{IndexSidecarDescriptor, SidecarKind};
use kyma_core::tenant::DEFAULT_TENANT;
use kyma_core::types::TableId;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

fn vec_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 4),
            true,
        ),
    ]))
}

fn scalar_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("msg", DataType::Utf8, true),
    ]))
}

async fn register_extent(
    catalog: &PostgresCatalog,
    table_id: TableId,
    tref: &kyma_core::catalog::TableRef,
) -> kyma_core::types::ExtentId {
    let id = kyma_core::types::ExtentId::from_uuid(Uuid::new_v4());
    let manifest = ExtentManifest {
        id,
        table_id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: format!("{DEFAULT_TENANT}/extents/{}.kyma", id.as_uuid()),
        byte_size: 1024,
        row_count: 100,
        min_timestamp: None,
        max_timestamp: None,
        column_stats: serde_json::json!({}),
        present_paths: vec![],
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    };
    let mut txn = catalog.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(manifest).await.unwrap();
    txn.commit(SnapshotSummary {
        rows_added: 100,
        operation: "ingest".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn enqueues_builds_for_unindexed_vector_extents_only() {
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pg = PostgresCatalog::connect(&url)
        .await
        .expect("connect + migrate");

    // A table WITH a vector column + 2 extents.
    let db = pg.create_database("default").await.unwrap();
    pg.create_table(db, "vecs", vec_schema(), TableConfig::default())
        .await
        .unwrap();
    let vtref = pg.lookup_table("default", "vecs").await.unwrap();
    // Both extents carry vec stats (embeddings present) so they're eligible for
    // ivf_rabitq indexing.
    let vstats = serde_json::json!({"embedding": {"vec": {"centroid": [0.0,0.0,0.0,0.0], "radius": 0.0, "count": 100, "metric": "cosine"}}});
    let e1 = register_extent_stats(&pg, vtref.id, &vtref, vstats.clone()).await;
    let _e2 = register_extent_stats(&pg, vtref.id, &vtref, vstats).await;
    // Re-read the table ref so current_snapshot_id reflects both commits.
    let vtref = pg.lookup_table("default", "vecs").await.unwrap();

    // A table with NO vector column.
    pg.create_table(db, "logs", scalar_schema(), TableConfig::default())
        .await
        .unwrap();
    let ltref = pg.lookup_table("default", "logs").await.unwrap();
    let _ = register_extent(&pg, ltref.id, &ltref).await;

    let catalog: Arc<dyn Catalog> = Arc::new(pg.clone());
    let sched = IndexScheduler::new(catalog.clone());

    // First tick: 2 un-indexed vector extents → ≥1 job (bundled, default
    // max_extents_per_job=32 → exactly 1 job covering both). The scalar table
    // contributes nothing.
    let n1 = sched.tick().await.unwrap();
    assert_eq!(
        n1, 1,
        "one bundled ivf_rabitq job for the two vector extents"
    );

    // Register a sidecar for BOTH extents → next tick finds nothing missing.
    for eid in [e1, _e2] {
        pg.register_index_sidecar(
            DEFAULT_TENANT,
            &IndexSidecarDescriptor {
                id: Uuid::new_v4(),
                extent_id: eid,
                table_id: vtref.id,
                column: "embedding".into(),
                kind: SidecarKind::IvfRabitq,
                object_path: format!(
                    "{DEFAULT_TENANT}/indexes/{}/embedding.ivf_rabitq",
                    eid.as_uuid()
                ),
                byte_size: 10,
                params: serde_json::json!({}),
                embedding_model_id: Some("test/model".into()),
                created_at: chrono::Utc::now(),
            },
        )
        .await
        .unwrap();
    }

    let n2 = sched.tick().await.unwrap();
    assert_eq!(n2, 0, "all vector extents indexed → no new jobs");
}

/// Register an extent carrying explicit `column_stats`.
async fn register_extent_stats(
    catalog: &PostgresCatalog,
    table_id: TableId,
    tref: &kyma_core::catalog::TableRef,
    column_stats: serde_json::Value,
) -> kyma_core::types::ExtentId {
    let id = kyma_core::types::ExtentId::from_uuid(Uuid::new_v4());
    let manifest = ExtentManifest {
        id,
        table_id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: format!("{DEFAULT_TENANT}/extents/{}.kyma", id.as_uuid()),
        byte_size: 1024,
        row_count: 100,
        min_timestamp: None,
        max_timestamp: None,
        column_stats,
        present_paths: vec![],
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    };
    let mut txn = catalog.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(manifest).await.unwrap();
    txn.commit(SnapshotSummary {
        rows_added: 100,
        operation: "ingest".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn enqueues_embed_backfill_for_unembedded_extents_only() {
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pg = PostgresCatalog::connect(&url)
        .await
        .expect("connect + migrate");

    // Table with a text source column + a vector embedding column.
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("msg", DataType::Utf8, true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 4),
            true,
        ),
    ]));
    let db = pg.create_database("default").await.unwrap();
    pg.create_table(db, "docs", schema, TableConfig::default())
        .await
        .unwrap();
    let tref = pg.lookup_table("default", "docs").await.unwrap();

    // Configure auto-embed: msg → embedding.
    pg.set_table_embed_config(
        DEFAULT_TENANT,
        &kyma_core::catalog::TableEmbedConfig {
            table_id: tref.id,
            source_column: "msg".into(),
            embedding_column: "embedding".into(),
            model_id: "fake/model".into(),
            dim: 4,
            auto_embed: true,
        },
    )
    .await
    .unwrap();

    // One extent with NO vec stats for the embedding column → needs embedding.
    let _e1 = register_extent_stats(&pg, tref.id, &tref, serde_json::json!({})).await;
    let tref = pg.lookup_table("default", "docs").await.unwrap();

    let sched = IndexScheduler::new(Arc::new(pg.clone()) as Arc<dyn Catalog>);
    // tick enqueues embed_backfill (the extent has no embeddings yet). It does
    // NOT also enqueue ivf_rabitq for the same extent — that has no vec stats.
    let n1 = sched.tick().await.unwrap();
    assert_eq!(n1, 1, "one embed_backfill job for the un-embedded extent");

    // A second extent that ALREADY has vec stats (as if backfilled) → no embed
    // job for it; only ivf_rabitq (it has vec stats but no sidecar).
    let _e2 = register_extent_stats(
        &pg,
        tref.id,
        &tref,
        serde_json::json!({"embedding": {"vec": {"centroid": [0.0,0.0,0.0,0.0], "radius": 0.0, "count": 100, "metric": "cosine"}}}),
    )
    .await;

    let sched2 = IndexScheduler::new(Arc::new(pg.clone()) as Arc<dyn Catalog>);
    let n2 = sched2.tick().await.unwrap();
    // e1 still needs embed (1) + e2 needs ivf_rabitq (1) = 2.
    assert_eq!(n2, 2, "embed for e1 + ivf_rabitq for the embedded e2");
}
