//! Local-mode (SQLite) index/embed activation.
//!
//! The server enqueues `embed_backfill` / `index_build` jobs into the Postgres
//! fabric queue; local mode has no Postgres. This test proves the SQLite path
//! the embedded worker wires up (S1.5):
//!
//!   `IndexScheduler::with_enqueuer(catalog, SqliteEnqueuer)` scans tables over
//!   a SQLite catalog and enqueues the right jobs into the SQLite `jobs` table,
//!   and `SqliteQueue` (the same queue the local fabric worker drains) can claim
//!   them.
//!
//! It mirrors the Postgres `pensieve-compaction::index_scheduler_it` over the SQLite
//! backend — the scheduler's scan logic is catalog-agnostic (only `Catalog`
//! trait methods), and only the enqueue target differs. The executors that
//! actually build sidecars are covered by `pensieve-jobs::embed_backfill_it` and
//! `index_build_test` (both SQLite), and the full real-model serve→ingest→search
//! path by `scripts/fresh-install-validate.sh`.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema};
use pensieve_catalog_sqlite::SqliteCatalog;
use pensieve_compaction::IndexScheduler;
use pensieve_core::catalog::{
    Catalog, ExtentManifest, SnapshotSummary, TableConfig, TableEmbedConfig, TableRef,
};
use pensieve_core::tenant::DEFAULT_TENANT;
use pensieve_core::types::{ExtentId, TableId};
use pensieve_jobs::JobQueue;
use pensieve_local::sqlite_queue::{SqliteEnqueuer, SqliteQueue};
use uuid::Uuid;

/// `(id Int64, msg Utf8, embedding FixedSizeList<Float32, 4>)` — a text source
/// column + a vector embedding column, the shape `set_table_embed_config` wires.
fn docs_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("msg", DataType::Utf8, true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 4),
            true,
        ),
    ]))
}

/// Register an extent carrying explicit `column_stats` (no real bytes needed —
/// the scheduler reads only catalog metadata, exactly like the PG test).
async fn register_extent_stats(
    catalog: &SqliteCatalog,
    table_id: TableId,
    tref: &TableRef,
    column_stats: serde_json::Value,
) -> ExtentId {
    let id = ExtentId::from_uuid(Uuid::new_v4());
    let manifest = ExtentManifest {
        id,
        table_id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: format!("{DEFAULT_TENANT}/extents/{}.pensieve", id.as_uuid()),
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

/// Create `docs`, declare auto-embed `msg → embedding`, return its `TableRef`.
async fn setup_docs(catalog: &SqliteCatalog) -> TableRef {
    let db = catalog.create_database("default").await.unwrap();
    catalog
        .create_table(db, "docs", docs_schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table("default", "docs").await.unwrap();
    catalog
        .set_table_embed_config(
            DEFAULT_TENANT,
            &TableEmbedConfig {
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
    tref
}

/// Synthetic vec stats marking an extent's embedding column as populated.
fn vec_stats() -> serde_json::Value {
    serde_json::json!({"embedding": {"vec": {"centroid": [0.0,0.0,0.0,0.0], "radius": 0.0, "count": 100, "metric": "cosine"}}})
}

#[tokio::test]
async fn unembedded_extent_enqueues_embed_backfill_claimable_over_sqlite() {
    let catalog = SqliteCatalog::connect_in_memory().await.unwrap();
    let tref = setup_docs(&catalog).await;
    // One extent with NO vec stats on the embedding column → needs embedding.
    let _e1 = register_extent_stats(&catalog, tref.id, &tref, serde_json::json!({})).await;

    let pool = catalog.pool().clone();
    let scheduler = IndexScheduler::with_enqueuer(
        Arc::new(catalog.clone()),
        Arc::new(SqliteEnqueuer::new(pool.clone())),
    );

    // Enqueues exactly one embed_backfill (no ivf_rabitq: the extent has no vec
    // stats yet; no tantivy_fts: that's gated on embedded extents).
    let n = scheduler.tick().await.unwrap();
    assert_eq!(n, 1, "one embed_backfill job for the un-embedded extent");

    // The same queue the local worker drains can claim it under its kind.
    let queue = SqliteQueue::new(pool, Uuid::new_v4(), vec!["data_source".into()], 120);
    let embed = queue
        .claim(&[pensieve_core::fabric::JOB_EMBED_BACKFILL.to_string()], 10)
        .await
        .unwrap();
    assert_eq!(embed.len(), 1, "embed_backfill job is claimable");
    let index = queue
        .claim(&[pensieve_core::fabric::JOB_INDEX_BUILD.to_string()], 10)
        .await
        .unwrap();
    assert!(
        index.is_empty(),
        "no index_build jobs yet (nothing embedded)"
    );
}

#[tokio::test]
async fn embedded_extent_enqueues_ann_and_fts_claimable_over_sqlite() {
    let catalog = SqliteCatalog::connect_in_memory().await.unwrap();
    let tref = setup_docs(&catalog).await;
    // An extent that already carries vec stats (as if embed_backfill ran) → it
    // gets BOTH an ivf_rabitq sidecar (vec stats, no sidecar) AND a tantivy_fts
    // sidecar on the source text column.
    let _e = register_extent_stats(&catalog, tref.id, &tref, vec_stats()).await;
    let tref = catalog.lookup_table("default", "docs").await.unwrap();

    let pool = catalog.pool().clone();
    let scheduler = IndexScheduler::with_enqueuer(
        Arc::new(catalog.clone()),
        Arc::new(SqliteEnqueuer::new(pool.clone())),
    );

    let n = scheduler.tick().await.unwrap();
    assert_eq!(n, 2, "ivf_rabitq + tantivy_fts for the embedded extent");

    // Both sidecar builds are enqueued under JOB_INDEX_BUILD and claimable.
    let queue = SqliteQueue::new(pool, Uuid::new_v4(), vec!["data_source".into()], 120);
    let index = queue
        .claim(&[pensieve_core::fabric::JOB_INDEX_BUILD.to_string()], 10)
        .await
        .unwrap();
    assert_eq!(index.len(), 2, "ivf_rabitq + tantivy_fts both claimable");
    // No embed needed — the extent is already embedded.
    let embed = queue
        .claim(&[pensieve_core::fabric::JOB_EMBED_BACKFILL.to_string()], 10)
        .await
        .unwrap();
    assert!(
        embed.is_empty(),
        "embedded extent enqueues no embed_backfill"
    );
}
