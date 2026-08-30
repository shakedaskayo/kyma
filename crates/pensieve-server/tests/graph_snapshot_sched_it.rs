//! Integration test for the graph-snapshot activation scheduler (S3.2).
//!
//! A registered graph's persistent CSR snapshot is built out of band by
//! `GraphSnapshotScheduler`, debounced on the edge table's `current_snapshot_id`.
//! We assert the full loop end-to-end against a real catalog + format:
//!   - first sweep builds a snapshot (object + freshness marker land in the
//!     object store, return count == 1),
//!   - a second sweep with no intervening write is debounced (count == 0,
//!     marker records the edge table's snapshot id),
//!   - the snapshot object is the consumer's key `snapshot_path(db, edge_table)`.

#![cfg(feature = "test-support")]

use arrow_schema::{DataType, Field, Schema};
use kyma_core::catalog::{GraphSpec, TableConfig};
use kyma_server::graph_snapshot_sched::GraphSnapshotScheduler;
use object_store::path::Path as ObjPath;
use std::sync::Arc;

/// Seed `obs` with `gnodes(id,name)` + `gedges(src,dst,type)` and register the
/// conventional graph "kg" over them (same shape as the cypher fixtures).
async fn seeded_state_with_graph() -> kyma_server::QueryState {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let obs_id = state
        .catalog
        .lookup_database("obs")
        .await
        .expect("lookup_database obs")
        .expect("obs database exists");

    let node_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    state
        .catalog
        .create_table(obs_id, "gnodes", node_schema, TableConfig::default())
        .await
        .expect("create_table gnodes");

    let edge_schema = Arc::new(Schema::new(vec![
        Field::new("src", DataType::Utf8, false),
        Field::new("dst", DataType::Utf8, false),
        Field::new("type", DataType::Utf8, true),
    ]));
    state
        .catalog
        .create_table(obs_id, "gedges", edge_schema, TableConfig::default())
        .await
        .expect("create_table gedges");

    let spec = GraphSpec::with_defaults("gnodes", "gedges");
    state
        .catalog
        .create_graph("obs", "kg", spec)
        .await
        .expect("create_graph kg");
    state
}

#[tokio::test]
async fn scheduler_builds_then_debounces_snapshot() {
    let state = seeded_state_with_graph().await;
    let store = state
        .format
        .object_store()
        .expect("test format has an object store");
    let sched = GraphSnapshotScheduler::new(state.catalog.clone(), state.format.clone());

    // First sweep: the snapshot has never been built → exactly one rebuild.
    let built = sched.tick().await.expect("first tick");
    assert_eq!(built, 1, "first sweep builds the single registered graph");

    // The snapshot object landed at the consumer's key, and a freshness marker
    // beside it records a non-empty source version.
    let snap_path = kyma_graph::snapshot::snapshot_path("obs", "gedges");
    store
        .head(&ObjPath::from(snap_path.as_str()))
        .await
        .expect("snapshot object exists at snapshot_path(db, edge_table)");
    let meta_path = kyma_graph::snapshot::meta_path("obs", "gedges");
    let marker = kyma_graph::snapshot::load_snapshot_meta(&store, &meta_path)
        .await
        .expect("read marker")
        .expect("marker written after a successful build");
    assert!(!marker.is_empty(), "marker records the source snapshot id");

    // Second sweep: nothing was written to gedges, so the live snapshot id still
    // matches the marker → debounced, no rebuild.
    let built2 = sched.tick().await.expect("second tick");
    assert_eq!(built2, 0, "unchanged edge table is debounced");
}
