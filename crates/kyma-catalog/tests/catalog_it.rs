//! Integration tests for `PostgresCatalog`.
//!
//! Uses `testcontainers` to spin up a real Postgres per test — no Docker
//! mocking, no transactional test harness. Verifies the full contract of
//! the `Catalog` trait including the snapshot-CAS conflict path.
//!
//! These run under `cargo test --test catalog_it`; they require Docker but
//! no other setup.

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{
    Catalog, ExtentManifest, NodeInfo, NodeRole, PrunePredicate, SnapshotSummary, TableConfig,
};
use kyma_core::errors::{CatalogError, Error};
use kyma_core::types::{ExtentId, SchemaSnapshotId, TableId};
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

// ------------------------------------------------------------------
// Fixture — one Postgres per test, catalog connected + migrated
// ------------------------------------------------------------------

struct Fixture {
    catalog: PostgresCatalog,
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn fixture() -> Fixture {
    // pgvector/pgvector:pg16 ships the `vector` extension that migration 004
    // creates; the bare postgres image lacks it and migration would fail.
    let container = Postgres::default()
        .with_user("kyma")
        .with_password("kyma_dev")
        .with_db_name("kyma")
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("failed to start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get mapped port");
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");
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
        Field::new("status", DataType::Int32, true),
        Field::new("body", DataType::Utf8, true),
    ]))
}

fn sample_manifest(table_id: TableId, schema_snap: SchemaSnapshotId) -> ExtentManifest {
    ExtentManifest {
        id: ExtentId::new(),
        table_id,
        schema_snapshot_id: schema_snap,
        object_path: format!("test/extents/{}.kyma", uuid::Uuid::new_v4()),
        byte_size: 1024,
        row_count: 100,
        min_timestamp: Some(chrono::Utc::now()),
        max_timestamp: Some(chrono::Utc::now()),
        column_stats: serde_json::json!({}),
        present_paths: vec![],
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    }
}

// ------------------------------------------------------------------
// Database + table lifecycle
// ------------------------------------------------------------------

#[tokio::test]
async fn create_database_and_table() {
    let fx = fixture().await;
    let db = fx.catalog.create_database("default").await.unwrap();
    let _ = fx
        .catalog
        .create_table(db, "t", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = fx.catalog.lookup_table("default", "t").await.unwrap();
    assert_eq!(tref.name, "t");
    assert_eq!(tref.schema.fields().len(), 3);
}

#[tokio::test]
async fn lookup_missing_table_returns_not_found() {
    let fx = fixture().await;
    fx.catalog.create_database("default").await.unwrap();
    let err = fx
        .catalog
        .lookup_table("default", "ghost")
        .await
        .unwrap_err();
    match err {
        Error::Catalog(CatalogError::TableNotFound { database, name }) => {
            assert_eq!(database, "default");
            assert_eq!(name, "ghost");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test]
async fn list_tables_in_database_orders_by_name() {
    let fx = fixture().await;
    let db = fx.catalog.create_database("default").await.unwrap();
    for name in ["beta", "alpha", "gamma"] {
        fx.catalog
            .create_table(db, name, sample_schema(), TableConfig::default())
            .await
            .unwrap();
    }
    let listed = fx.catalog.list_tables_in_database("default").await.unwrap();
    let names: Vec<_> = listed.iter().map(|t| t.name.clone()).collect();
    assert_eq!(names, vec!["alpha", "beta", "gamma"]);
}

// ------------------------------------------------------------------
// Snapshot commit — happy path
// ------------------------------------------------------------------

#[tokio::test]
async fn snapshot_commit_happy_path() {
    let fx = fixture().await;
    let db = fx.catalog.create_database("default").await.unwrap();
    let tid = fx
        .catalog
        .create_table(db, "t", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let table = fx.catalog.lookup_table("default", "t").await.unwrap();

    let mut txn = fx.catalog.begin_snapshot(tid).await.unwrap();
    let m = sample_manifest(tid, table.schema_snapshot_id);
    let extent_id = m.id;
    txn.add_extent(m).await.unwrap();
    let new_snap = txn
        .commit(SnapshotSummary {
            rows_added: 100,
            bytes_added: 1024,
            operation: "ingest".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    assert_ne!(new_snap, table.current_snapshot_id, "snapshot must advance");

    // The listed extent should round-trip with stable fields.
    let listed = fx
        .catalog
        .list_extents(tid, new_snap, &PrunePredicate::default())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, extent_id);
    assert_eq!(listed[0].row_count, 100);
    assert_eq!(listed[0].byte_size, 1024);
}

// ------------------------------------------------------------------
// Snapshot commit — CAS conflict resolution
// ------------------------------------------------------------------

#[tokio::test]
async fn snapshot_cas_conflict_bubbles_up() {
    let fx = fixture().await;
    let db = fx.catalog.create_database("default").await.unwrap();
    let tid = fx
        .catalog
        .create_table(db, "t", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let table = fx.catalog.lookup_table("default", "t").await.unwrap();

    // Open two transactions from the same parent snapshot. This is the
    // scenario two concurrent ingest workers will produce.
    let mut txn_a = fx.catalog.begin_snapshot(tid).await.unwrap();
    let mut txn_b = fx.catalog.begin_snapshot(tid).await.unwrap();
    assert_eq!(txn_a.parent_snapshot_id(), table.current_snapshot_id);
    assert_eq!(txn_b.parent_snapshot_id(), table.current_snapshot_id);

    txn_a
        .add_extent(sample_manifest(tid, table.schema_snapshot_id))
        .await
        .unwrap();
    txn_b
        .add_extent(sample_manifest(tid, table.schema_snapshot_id))
        .await
        .unwrap();

    // Commit A first — wins.
    let _new_snap = txn_a
        .commit(SnapshotSummary {
            operation: "ingest".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    // Commit B — must fail with CatalogError::Conflict.
    let err = txn_b
        .commit(SnapshotSummary {
            operation: "ingest".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    match err {
        Error::Catalog(CatalogError::Conflict) => { /* expected */ }
        other => panic!("expected CatalogError::Conflict, got {other:?}"),
    }

    // After the conflict, state should still be consistent: only one extent
    // visible, snapshot pointer advanced exactly once past bootstrap.
    let current = fx.catalog.lookup_table("default", "t").await.unwrap();
    let listed = fx
        .catalog
        .list_extents(tid, current.current_snapshot_id, &PrunePredicate::default())
        .await
        .unwrap();
    assert_eq!(listed.len(), 1, "exactly one extent survives the race");
}

// ------------------------------------------------------------------
// Retry-after-conflict — simulating what the real WritePath does
// ------------------------------------------------------------------

#[tokio::test]
async fn conflict_is_resolvable_by_retry() {
    let fx = fixture().await;
    let db = fx.catalog.create_database("default").await.unwrap();
    let tid = fx
        .catalog
        .create_table(db, "t", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let table = fx.catalog.lookup_table("default", "t").await.unwrap();

    // First committer wins.
    let mut txn_a = fx.catalog.begin_snapshot(tid).await.unwrap();
    txn_a
        .add_extent(sample_manifest(tid, table.schema_snapshot_id))
        .await
        .unwrap();
    txn_a
        .commit(SnapshotSummary {
            operation: "ingest".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    // Second committer retries: re-reads current snapshot, re-lineages.
    let mut tries = 0;
    let final_snap = loop {
        tries += 1;
        let mut txn = fx.catalog.begin_snapshot(tid).await.unwrap();
        txn.add_extent(sample_manifest(tid, table.schema_snapshot_id))
            .await
            .unwrap();
        match txn
            .commit(SnapshotSummary {
                operation: "ingest".into(),
                ..Default::default()
            })
            .await
        {
            Ok(s) => break s,
            Err(Error::Catalog(CatalogError::Conflict)) if tries < 5 => continue,
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    };

    // Two extents now visible.
    let listed = fx
        .catalog
        .list_extents(tid, final_snap, &PrunePredicate::default())
        .await
        .unwrap();
    assert_eq!(listed.len(), 2);
}

// ------------------------------------------------------------------
// Node identity + heartbeat
// ------------------------------------------------------------------

#[tokio::test]
async fn node_register_heartbeat_deregister() {
    let fx = fixture().await;
    let lease = fx
        .catalog
        .register_node(NodeInfo {
            role: NodeRole::AllInOne,
            endpoint: "127.0.0.1:8080".into(),
            capabilities: serde_json::json!({"version": "test"}),
        })
        .await
        .unwrap();

    // Heartbeat should succeed.
    fx.catalog.heartbeat(&lease).await.unwrap();

    // Heartbeat against a different lease id should fail.
    let stolen = kyma_core::catalog::NodeLease {
        node_id: lease.node_id,
        lease_id: uuid::Uuid::new_v4(),
        expires_at: lease.expires_at,
    };
    let err = fx.catalog.heartbeat(&stolen).await.unwrap_err();
    // Any error is fine — we just want to confirm stolen-lease rejection.
    assert!(matches!(err, Error::Catalog(_)), "got: {err:?}");

    // Deregister cleans the row up.
    fx.catalog.deregister_node(lease.clone()).await.unwrap();
    let err2 = fx.catalog.heartbeat(&lease).await.unwrap_err();
    assert!(matches!(err2, Error::Catalog(_)));
}

// ------------------------------------------------------------------
// list_extents prune predicates
// ------------------------------------------------------------------

#[tokio::test]
async fn list_extents_honors_time_range() {
    let fx = fixture().await;
    let db = fx.catalog.create_database("default").await.unwrap();
    let tid = fx
        .catalog
        .create_table(db, "t", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let table = fx.catalog.lookup_table("default", "t").await.unwrap();

    let base = chrono::Utc::now() - chrono::Duration::hours(10);
    let make_extent = |hour_offset: i64| ExtentManifest {
        id: ExtentId::new(),
        table_id: tid,
        schema_snapshot_id: table.schema_snapshot_id,
        object_path: format!("test/{}.kyma", uuid::Uuid::new_v4()),
        byte_size: 100,
        row_count: 10,
        min_timestamp: Some(base + chrono::Duration::hours(hour_offset)),
        max_timestamp: Some(
            base + chrono::Duration::hours(hour_offset) + chrono::Duration::minutes(30),
        ),
        column_stats: serde_json::json!({}),
        present_paths: vec![],
        compaction_gen: 0,
        created_at: chrono::Utc::now(),
    };

    // Commit three extents at hour 0, 3, 6 (non-overlapping).
    for offset in [0i64, 3, 6] {
        let mut txn = fx.catalog.begin_snapshot(tid).await.unwrap();
        txn.add_extent(make_extent(offset)).await.unwrap();
        txn.commit(SnapshotSummary {
            operation: "ingest".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    }

    let latest = fx.catalog.lookup_table("default", "t").await.unwrap();

    // Time-range window covering only hour 2-4 should match the middle extent only.
    let prune = PrunePredicate {
        time_range: Some(kyma_core::catalog::TimeRange {
            start_inclusive: base + chrono::Duration::hours(2),
            end_exclusive: base + chrono::Duration::hours(4),
        }),
        ..Default::default()
    };
    let listed = fx
        .catalog
        .list_extents(tid, latest.current_snapshot_id, &prune)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1, "only hour-3 extent should match");
}

// ------------------------------------------------------------------
// External-identity (JIT) user provisioning — Supabase auth backend contract
// ------------------------------------------------------------------

#[tokio::test]
async fn upsert_external_user_creates_then_refreshes() {
    let fx = fixture().await;
    let tenant = kyma_core::tenant::DEFAULT_TENANT;

    let u1 = fx
        .catalog
        .upsert_external_user_in_tenant(tenant, "supabase", "sub-123", "alice@example.com", "read")
        .await
        .unwrap();
    assert_eq!(u1.username, "alice@example.com");
    assert_eq!(u1.role, "read");

    // Same identity, new email + elevated role: updates in place.
    let u2 = fx
        .catalog
        .upsert_external_user_in_tenant(tenant, "supabase", "sub-123", "alice@corp.com", "admin")
        .await
        .unwrap();
    assert_eq!(u2.id, u1.id, "same external identity → same user row");
    assert_eq!(u2.username, "alice@corp.com");
    assert_eq!(u2.role, "admin");
    assert_eq!(fx.catalog.count_users().await.unwrap(), 1);

    // Different subject → different user; password users are unaffected.
    fx.catalog
        .create_user("admin", "$argon2id$fake", "admin")
        .await
        .unwrap();
    let u3 = fx
        .catalog
        .upsert_external_user_in_tenant(tenant, "supabase", "sub-456", "bob@example.com", "read")
        .await
        .unwrap();
    assert_ne!(u3.id, u1.id);
    assert_eq!(fx.catalog.count_users().await.unwrap(), 3);
}
