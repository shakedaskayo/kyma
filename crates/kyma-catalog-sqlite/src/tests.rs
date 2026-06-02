//! In-memory round-trip tests for the SQLite catalog. These exercise the
//! Iceberg-style metadata path (db → table → snapshot → extent), Rust-side
//! extent pruning, schema evolution, graphs, tasks, users, tokens, dashboards,
//! and the idempotency ledger — no Postgres, no object store.

use super::*;
use arrow_schema::{DataType, Field, Schema};
use kyma_core::catalog::{
    ColumnPrune, ExtentManifest, GraphSpec, IngestLedgerEntry, NodeInfo, PrunePredicate,
    SnapshotSummary, TableConfig,
};
use std::collections::HashMap;

fn sample_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("ts", DataType::Timestamp(arrow_schema::TimeUnit::Nanosecond, None), true),
        Field::new("level", DataType::Utf8, true),
        Field::new("msg", DataType::Utf8, true),
    ]))
}

fn extent_for(tref: &TableRef, distinct_levels: &[&str]) -> ExtentManifest {
    let levels: Vec<Json> = distinct_levels.iter().map(|s| Json::String((*s).into())).collect();
    ExtentManifest {
        id: ExtentId::new(),
        table_id: tref.id,
        schema_snapshot_id: tref.schema_snapshot_id,
        object_path: format!("ext/{}.tlm", Uuid::new_v4()),
        byte_size: 4096,
        row_count: 100,
        min_timestamp: Some(Utc::now() - chrono::Duration::minutes(10)),
        max_timestamp: Some(Utc::now()),
        column_stats: serde_json::json!({
            "level": { "distinct": levels },
            "embedding": { "vec": { "centroid": [1.0, 0.0, 0.0], "radius": 0.0 } }
        }),
        present_paths: vec!["ts".into(), "level".into(), "msg".into()],
        compaction_gen: 0,
        created_at: Utc::now(),
    }
}

async fn fresh() -> SqliteCatalog {
    SqliteCatalog::connect_in_memory().await.expect("open in-memory catalog")
}

#[tokio::test]
async fn full_table_snapshot_extent_round_trip() {
    let cat = fresh().await;

    // create db + table
    let db = cat.create_database("default").await.unwrap();
    let table_id = cat
        .create_table(db, "logs", sample_schema(), TableConfig::default())
        .await
        .unwrap();

    // lookup resolves schema + pointers
    let tref = cat.lookup_table("default", "logs").await.unwrap();
    assert_eq!(tref.id, table_id);
    assert_eq!(tref.schema.fields().len(), 3);

    // begin snapshot, add two extents, commit
    let mut txn = cat.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(extent_for(&tref, &["info", "warn"])).await.unwrap();
    txn.add_extent(extent_for(&tref, &["error"])).await.unwrap();
    let new_snap = txn
        .commit(SnapshotSummary { operation: "ingest".into(), rows_added: 200, ..Default::default() })
        .await
        .unwrap();

    // table now points at the new snapshot
    let tref = cat.lookup_table("default", "logs").await.unwrap();
    assert_eq!(tref.current_snapshot_id, new_snap);

    // list all live extents (no prune)
    let all = cat
        .list_extents(table_id, tref.current_snapshot_id, &PrunePredicate::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 2, "both extents visible");

    // newest-first ordering by min_timestamp
    assert!(all[0].min_timestamp >= all[1].min_timestamp);
}

#[tokio::test]
async fn equals_prune_drops_non_matching_extent() {
    let cat = fresh().await;
    let db = cat.create_database("default").await.unwrap();
    let table_id = cat
        .create_table(db, "logs", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = cat.lookup_table("default", "logs").await.unwrap();

    let mut txn = cat.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(extent_for(&tref, &["info", "warn"])).await.unwrap();
    txn.add_extent(extent_for(&tref, &["error"])).await.unwrap();
    txn.commit(SnapshotSummary { operation: "ingest".into(), ..Default::default() })
        .await
        .unwrap();
    let tref = cat.lookup_table("default", "logs").await.unwrap();

    // Equals("error") should keep only the extent whose distinct set has it.
    let mut preds = HashMap::new();
    preds.insert("level".to_string(), ColumnPrune::Equals(Json::String("error".into())));
    let pruned = cat
        .list_extents(
            table_id,
            tref.current_snapshot_id,
            &PrunePredicate { column_predicates: preds, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(pruned.len(), 1, "only the error extent survives");

    // A value present in neither distinct set drops both.
    let mut preds = HashMap::new();
    preds.insert("level".to_string(), ColumnPrune::Equals(Json::String("trace".into())));
    let none = cat
        .list_extents(
            table_id,
            tref.current_snapshot_id,
            &PrunePredicate { column_predicates: preds, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(none.len(), 0, "no extent has level=trace");
}

#[tokio::test]
async fn vector_distance_prune_uses_centroid_radius() {
    let cat = fresh().await;
    let db = cat.create_database("default").await.unwrap();
    let table_id = cat
        .create_table(db, "mem", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = cat.lookup_table("default", "mem").await.unwrap();

    let mut txn = cat.begin_snapshot(table_id).await.unwrap();
    txn.add_extent(extent_for(&tref, &["info"])).await.unwrap(); // centroid = [1,0,0], radius 0
    txn.commit(SnapshotSummary::default()).await.unwrap();
    let tref = cat.lookup_table("default", "mem").await.unwrap();

    // Query == centroid → lower bound 0 < threshold ⇒ kept.
    let mut preds = HashMap::new();
    preds.insert(
        "embedding".to_string(),
        ColumnPrune::VectorDistance { query: vec![1.0, 0.0, 0.0], threshold: 0.5 },
    );
    let kept = cat
        .list_extents(
            table_id,
            tref.current_snapshot_id,
            &PrunePredicate { column_predicates: preds, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(kept.len(), 1);

    // Opposite query → lower bound 2.0, threshold tiny ⇒ pruned.
    let mut preds = HashMap::new();
    preds.insert(
        "embedding".to_string(),
        ColumnPrune::VectorDistance { query: vec![-1.0, 0.0, 0.0], threshold: 0.01 },
    );
    let dropped = cat
        .list_extents(
            table_id,
            tref.current_snapshot_id,
            &PrunePredicate { column_predicates: preds, ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(dropped.len(), 0, "far extent pruned by ANN bound");
}

#[tokio::test]
async fn alter_table_add_column_evolves_schema() {
    let cat = fresh().await;
    let db = cat.create_database("default").await.unwrap();
    let table_id = cat
        .create_table(db, "logs", sample_schema(), TableConfig::default())
        .await
        .unwrap();

    cat.alter_table_add_column(table_id, "trace_id", "string").await.unwrap();

    let cols = cat.get_table_columns("default", "logs").await.unwrap();
    assert_eq!(cols.len(), 4);
    assert!(cols.iter().any(|c| c.name == "trace_id" && c.r#type == "string"));

    // Duplicate add is rejected.
    let dup = cat.alter_table_add_column(table_id, "trace_id", "string").await;
    assert!(dup.is_err(), "duplicate column rejected");
}

#[tokio::test]
async fn graphs_round_trip() {
    let cat = fresh().await;
    cat.create_database("memory").await.unwrap();
    let reg = cat
        .create_graph("memory", "mem_graph", GraphSpec::with_defaults("memory_nodes", "memory_edges"))
        .await
        .unwrap();
    assert_eq!(reg.name, "mem_graph");

    let got = cat.get_graph("memory", "mem_graph").await.unwrap().expect("graph present");
    assert_eq!(got.node_table, "memory_nodes");
    assert_eq!(got.edge_table, "memory_edges");

    let listed = cat.list_graphs("memory").await.unwrap();
    assert_eq!(listed.len(), 1);

    assert!(cat.drop_graph("memory", "mem_graph").await.unwrap());
    assert!(cat.get_graph("memory", "mem_graph").await.unwrap().is_none());
}

#[tokio::test]
async fn users_round_trip() {
    let cat = fresh().await;
    assert_eq!(cat.count_users().await.unwrap(), 0);
    cat.create_user("admin", "hash123", "admin").await.unwrap();
    assert_eq!(cat.count_users().await.unwrap(), 1);

    let (user, hash) = cat.get_user_with_hash("admin").await.unwrap().expect("user exists");
    assert_eq!(user.role, "admin");
    assert_eq!(hash, "hash123");

    assert!(cat.set_user_role("admin", "viewer").await.unwrap());
    let (user, _) = cat.get_user_with_hash("admin").await.unwrap().unwrap();
    assert_eq!(user.role, "viewer");

    assert!(cat.delete_user("admin").await.unwrap());
    assert_eq!(cat.count_users().await.unwrap(), 0);
}

#[tokio::test]
async fn api_token_lookup_and_revoke() {
    let cat = fresh().await;
    let hash = b"token-hash-bytes";
    cat.insert_api_token(hash, "admin", Some("svc"), "api", None).await.unwrap();

    let principal = cat.lookup_api_token(hash).await.unwrap().expect("token resolves");
    assert_eq!(principal.role, "admin");
    assert_eq!(principal.subject.as_deref(), Some("svc"));

    assert!(cat.revoke_api_token(hash).await.unwrap());
    assert!(cat.lookup_api_token(hash).await.unwrap().is_none(), "revoked token gone");
}

#[tokio::test]
async fn idempotency_ledger_is_write_once() {
    let cat = fresh().await;
    let db = cat.create_database("default").await.unwrap();
    let table_id = cat
        .create_table(db, "logs", sample_schema(), TableConfig::default())
        .await
        .unwrap();
    let tref = cat.lookup_table("default", "logs").await.unwrap();

    let entry = IngestLedgerEntry {
        table_id,
        snapshot_id: tref.current_snapshot_id,
        rows_ingested: 10,
        bytes_written: 1024,
        applied_at: Utc::now(),
    };
    let first = cat
        .record_idempotency("k1", entry.clone(), chrono::Duration::hours(1))
        .await
        .unwrap();
    assert!(first.is_some(), "first record stored");

    // Second record under same key is a no-op (raced/duplicate).
    let second = cat
        .record_idempotency("k1", entry, chrono::Duration::hours(1))
        .await
        .unwrap();
    assert!(second.is_none(), "duplicate key ignored");

    let looked = cat.lookup_idempotency("k1").await.unwrap().expect("entry present");
    assert_eq!(looked.rows_ingested, 10);
}

#[tokio::test]
async fn background_tasks_claim_complete() {
    let cat = fresh().await;
    let node = NodeId::new();
    cat.submit_task("compaction", None, serde_json::json!({"x": 1}), 5).await.unwrap();

    let claimed = cat
        .claim_task("compaction", node, chrono::Duration::minutes(5))
        .await
        .unwrap()
        .expect("a task is claimable");
    assert_eq!(claimed.kind, "compaction");
    assert_eq!(claimed.attempt, 1);

    // Nothing else to claim now.
    let none = cat.claim_task("compaction", node, chrono::Duration::minutes(5)).await.unwrap();
    assert!(none.is_none(), "claimed task not re-handed-out before lease expiry");

    cat.complete_task(claimed.id).await.unwrap();
}

#[tokio::test]
async fn node_register_heartbeat_list() {
    let cat = fresh().await;
    let lease = cat
        .register_node(NodeInfo {
            role: NodeRole::AllInOne,
            endpoint: "127.0.0.1:50051".into(),
            capabilities: serde_json::json!({}),
        })
        .await
        .unwrap();
    cat.heartbeat(&lease).await.unwrap();
    let live = cat.list_live_nodes(60).await.unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].endpoint, "127.0.0.1:50051");
    cat.deregister_node(lease).await.unwrap();
    assert_eq!(cat.list_live_nodes(60).await.unwrap().len(), 0);
}

#[tokio::test]
async fn dashboards_round_trip() {
    let cat = fresh().await;
    let dash = cat.create_dashboard("Ops", Some("ops board")).await.unwrap();
    let with_panels = cat.get_dashboard(dash.id).await.unwrap().expect("dashboard present");
    assert_eq!(with_panels.dashboard.name, "Ops");
    assert_eq!(with_panels.panels.len(), 0);

    let listed = cat.list_dashboards().await.unwrap();
    assert_eq!(listed.len(), 1);

    assert!(cat.delete_dashboard(dash.id).await.unwrap());
    assert!(cat.get_dashboard(dash.id).await.unwrap().is_none());
}
