//! Index-sidecar catalog lifecycle (migration 028): register → list →
//! idempotent re-register (upsert on the (extent, column, kind, model) key) →
//! cascade delete with the extent row → `delete_for_extents` returning object
//! paths for GC. Backs the S0.4 sidecar plumbing the S1 ANN/FTS builders
//! ride on.

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use pensieve_catalog::PostgresCatalog;
use pensieve_core::catalog::{Catalog, ExtentManifest, SnapshotSummary, TableConfig};
use pensieve_core::index_sidecar::{IndexSidecarDescriptor, SidecarKind};
use pensieve_core::tenant::{TenantId, DEFAULT_TENANT};
use pensieve_core::types::{ExtentId, TableId};
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

struct Fixture {
    catalog: PostgresCatalog,
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn fixture() -> Fixture {
    let container = Postgres::default()
        .with_user("pensieve")
        .with_password("pensieve_dev")
        .with_db_name("pensieve")
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("failed to start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get mapped port");
    let url = format!("postgres://pensieve:pensieve_dev@localhost:{port}/pensieve");
    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("catalog connect + migrate");
    Fixture {
        catalog,
        _container: container,
    }
}

fn sample_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            true,
        ),
        Field::new("body", DataType::Utf8, true),
    ]))
}

/// Create db + table + one committed extent; returns (`table_id`, `extent_id`).
async fn table_with_extent(catalog: &dyn Catalog, db: &str) -> (TableId, ExtentId) {
    let db_id = catalog.create_database(db).await.unwrap();
    let table_id = catalog
        .create_table(db_id, "logs", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = catalog.lookup_table(db, "logs").await.unwrap();
    let extent_id = ExtentId::new();
    let manifest = ExtentManifest {
        id: extent_id,
        table_id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: format!("test/extents/{}.pnsv", Uuid::new_v4()),
        byte_size: 1024,
        row_count: 100,
        min_timestamp: Some(chrono::Utc::now()),
        max_timestamp: Some(chrono::Utc::now()),
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
    (table_id, extent_id)
}

fn desc(
    table_id: TableId,
    extent_id: ExtentId,
    column: &str,
    kind: SidecarKind,
    model: Option<&str>,
) -> IndexSidecarDescriptor {
    IndexSidecarDescriptor {
        id: Uuid::new_v4(),
        extent_id,
        table_id,
        column: column.to_string(),
        kind,
        object_path: format!(
            "{DEFAULT_TENANT}/indexes/{extent_id}/{column}.{}",
            kind.as_str()
        ),
        byte_size: 2048,
        params: serde_json::json!({ "nlist": 16 }),
        embedding_model_id: model.map(str::to_string),
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn ann_tree_upsert_get_delete() {
    use pensieve_core::index_sidecar::AnnTreeDescriptor;
    let fx = fixture().await;
    let (table_id, _extent) = table_with_extent(&fx.catalog, "default").await;

    let mk = |gen: i64, fp: &str| AnnTreeDescriptor {
        id: Uuid::new_v4(),
        table_id,
        column: "embedding".into(),
        embedding_model_id: Some("bge@384".into()),
        generation: gen,
        extent_fingerprint: fp.into(),
        object_path: format!("{DEFAULT_TENANT}/ann_tree/{table_id}/embedding.kygt"),
        byte_size: 4096,
        params: serde_json::json!({ "global_nlist": 32, "dim": 384 }),
        created_at: chrono::Utc::now(),
    };

    // Insert.
    fx.catalog
        .upsert_ann_tree(DEFAULT_TENANT, &mk(1, "fp-a"))
        .await
        .unwrap();
    let got = fx
        .catalog
        .get_ann_tree(DEFAULT_TENANT, table_id, "embedding", Some("bge@384"))
        .await
        .unwrap()
        .expect("tree row present");
    assert_eq!(got.generation, 1);
    assert_eq!(got.extent_fingerprint, "fp-a");
    assert_eq!(got.byte_size, 4096);

    // Upsert in place (same table/column/model key) bumps generation + fingerprint.
    fx.catalog
        .upsert_ann_tree(DEFAULT_TENANT, &mk(2, "fp-b"))
        .await
        .unwrap();
    let got2 = fx
        .catalog
        .get_ann_tree(DEFAULT_TENANT, table_id, "embedding", Some("bge@384"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got2.generation, 2, "upsert replaces, not duplicates");
    assert_eq!(got2.extent_fingerprint, "fp-b");

    // A different model is a distinct row.
    let mut other = mk(1, "fp-c");
    other.embedding_model_id = Some("nomic@768".into());
    fx.catalog
        .upsert_ann_tree(DEFAULT_TENANT, &other)
        .await
        .unwrap();
    assert!(fx
        .catalog
        .get_ann_tree(DEFAULT_TENANT, table_id, "embedding", Some("nomic@768"))
        .await
        .unwrap()
        .is_some());

    // Missing model → None.
    assert!(fx
        .catalog
        .get_ann_tree(
            DEFAULT_TENANT,
            table_id,
            "embedding",
            Some("does-not-exist")
        )
        .await
        .unwrap()
        .is_none());

    // Delete returns both models' object paths.
    let paths = fx
        .catalog
        .delete_ann_tree(DEFAULT_TENANT, table_id, "embedding")
        .await
        .unwrap();
    assert_eq!(paths.len(), 2, "both model rows deleted");
    assert!(fx
        .catalog
        .get_ann_tree(DEFAULT_TENANT, table_id, "embedding", Some("bge@384"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn register_list_and_idempotent_reregister() {
    let fx = fixture().await;
    let (table_id, extent_id) = table_with_extent(&fx.catalog, "default").await;

    let ann = desc(
        table_id,
        extent_id,
        "embedding",
        SidecarKind::IvfRabitq,
        Some("m1"),
    );
    let fts = desc(table_id, extent_id, "body", SidecarKind::TantivyFts, None);
    fx.catalog
        .register_index_sidecar(DEFAULT_TENANT, &ann)
        .await
        .unwrap();
    fx.catalog
        .register_index_sidecar(DEFAULT_TENANT, &fts)
        .await
        .unwrap();

    // List everything for the extent.
    let all = fx
        .catalog
        .list_index_sidecars(DEFAULT_TENANT, table_id, &[extent_id], None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // Kind filter narrows to one.
    let only_fts = fx
        .catalog
        .list_index_sidecars(
            DEFAULT_TENANT,
            table_id,
            &[extent_id],
            Some(SidecarKind::TantivyFts),
        )
        .await
        .unwrap();
    assert_eq!(only_fts.len(), 1);
    assert_eq!(only_fts[0].column, "body");
    assert_eq!(only_fts[0].kind, SidecarKind::TantivyFts);
    assert!(only_fts[0].embedding_model_id.is_none());

    // Re-register the same logical sidecar (same extent/column/kind/model,
    // NULL model included) — upsert in place, no duplicate row, new fields win.
    let mut ann2 = desc(
        table_id,
        extent_id,
        "embedding",
        SidecarKind::IvfRabitq,
        Some("m1"),
    );
    ann2.byte_size = 9999;
    ann2.params = serde_json::json!({ "nlist": 64 });
    fx.catalog
        .register_index_sidecar(DEFAULT_TENANT, &ann2)
        .await
        .unwrap();
    let mut fts2 = desc(table_id, extent_id, "body", SidecarKind::TantivyFts, None);
    fts2.byte_size = 4242;
    fx.catalog
        .register_index_sidecar(DEFAULT_TENANT, &fts2)
        .await
        .unwrap();

    let all = fx
        .catalog
        .list_index_sidecars(DEFAULT_TENANT, table_id, &[extent_id], None)
        .await
        .unwrap();
    assert_eq!(
        all.len(),
        2,
        "re-register must upsert, not insert duplicates"
    );
    let ann_row = all
        .iter()
        .find(|d| d.kind == SidecarKind::IvfRabitq)
        .unwrap();
    assert_eq!(ann_row.byte_size, 9999);
    assert_eq!(ann_row.params, serde_json::json!({ "nlist": 64 }));
    let fts_row = all
        .iter()
        .find(|d| d.kind == SidecarKind::TantivyFts)
        .unwrap();
    assert_eq!(fts_row.byte_size, 4242);

    // A different model id for the same column+kind is a NEW sidecar.
    let ann_m2 = desc(
        table_id,
        extent_id,
        "embedding",
        SidecarKind::IvfRabitq,
        Some("m2"),
    );
    fx.catalog
        .register_index_sidecar(DEFAULT_TENANT, &ann_m2)
        .await
        .unwrap();
    let all = fx
        .catalog
        .list_index_sidecars(DEFAULT_TENANT, table_id, &[extent_id], None)
        .await
        .unwrap();
    assert_eq!(all.len(), 3, "distinct embedding models coexist");

    // Empty extent set lists nothing; foreign tenant sees nothing.
    assert!(fx
        .catalog
        .list_index_sidecars(DEFAULT_TENANT, table_id, &[], None)
        .await
        .unwrap()
        .is_empty());
    let t2 = TenantId::from_uuid(Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());
    assert!(fx
        .catalog
        .list_index_sidecars(t2, table_id, &[extent_id], None)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn cascade_delete_with_extent_row() {
    let fx = fixture().await;
    let (table_id, extent_id) = table_with_extent(&fx.catalog, "default").await;
    fx.catalog
        .register_index_sidecar(
            DEFAULT_TENANT,
            &desc(table_id, extent_id, "body", SidecarKind::TantivyFts, None),
        )
        .await
        .unwrap();

    // Hard-delete the extent row (the physical-GC path) — the sidecar row
    // must cascade away.
    fx.catalog.delete_extent_rows(&[extent_id]).await.unwrap();
    let left = fx
        .catalog
        .list_index_sidecars(DEFAULT_TENANT, table_id, &[extent_id], None)
        .await
        .unwrap();
    assert!(
        left.is_empty(),
        "extent_indexes rows must cascade with extents"
    );
}

#[tokio::test]
async fn delete_for_extents_returns_object_paths() {
    let fx = fixture().await;
    let (table_id, e1) = table_with_extent(&fx.catalog, "default").await;
    // Second extent on the same table.
    let tref = fx.catalog.lookup_table("default", "logs").await.unwrap();
    let e2 = ExtentId::new();
    let mut txn = fx.catalog.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(ExtentManifest {
        id: e2,
        table_id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: format!("test/extents/{}.pnsv", Uuid::new_v4()),
        byte_size: 512,
        row_count: 10,
        min_timestamp: None,
        max_timestamp: None,
        column_stats: serde_json::json!({}),
        present_paths: vec![],
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    })
    .await
    .unwrap();
    txn.commit(SnapshotSummary {
        operation: "ingest".into(),
        ..Default::default()
    })
    .await
    .unwrap();

    let d1 = desc(table_id, e1, "body", SidecarKind::TantivyFts, None);
    let d2 = desc(
        table_id,
        e1,
        "embedding",
        SidecarKind::IvfRabitq,
        Some("m1"),
    );
    let d3 = desc(table_id, e2, "body", SidecarKind::TantivyFts, None);
    for d in [&d1, &d2, &d3] {
        fx.catalog
            .register_index_sidecar(DEFAULT_TENANT, d)
            .await
            .unwrap();
    }

    // Deleting for e1 returns exactly e1's two paths; e2's sidecar survives.
    let mut paths = fx
        .catalog
        .delete_index_sidecars_for_extents(DEFAULT_TENANT, &[e1])
        .await
        .unwrap();
    paths.sort();
    let mut expected = vec![d1.object_path.clone(), d2.object_path.clone()];
    expected.sort();
    assert_eq!(paths, expected);

    let left = fx
        .catalog
        .list_index_sidecars(DEFAULT_TENANT, table_id, &[e1, e2], None)
        .await
        .unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].object_path, d3.object_path);

    // Idempotent: a second delete returns nothing.
    assert!(fx
        .catalog
        .delete_index_sidecars_for_extents(DEFAULT_TENANT, &[e1])
        .await
        .unwrap()
        .is_empty());
}
