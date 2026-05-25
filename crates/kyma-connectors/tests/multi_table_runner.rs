//! Unit tests for the multi-table output + graph auto-registration path.
//!
//! These tests use a recording RowSink and recording GraphRegisterFn (both
//! injected as closures) so we can assert on exactly what calls the runner
//! made without a live PostgresCatalog or WritePath.
//!
//! The critical property under test is **per-table idempotency-key
//! uniqueness**: the runner must produce a distinct key for each table in
//! `ConnectorRun::tables`, otherwise the second table's rows would be
//! silently deduplicated against the first table's.
//!
//! Because `ConnectorRunner::claim_and_run_one` currently requires a live
//! `PostgresCatalog` (it calls `claim_task` + SQL helpers), the unit tests
//! target the seam we *can* reach without a real Postgres instance: the
//! `RowSink` and `GraphRegisterFn` type aliases + the runner's multi-table
//! code path via a full integration test using testcontainers.
//!
//! The first test (`multi_table_distinct_idempotency_keys`) spins up a real
//! Postgres container and runs a fake two-table connector end-to-end via
//! `claim_and_run_one`. It verifies:
//! 1. The sink was called once per table (two calls for two tables).
//! 2. The idempotency keys for the two tables are DISTINCT (the critical
//!    invariant).
//! 3. `graph_register` was called exactly once.
//!
//! The second test (`legacy_rows_path_unchanged`) verifies that a connector
//! using only `rows` (not `tables`) still exercises the legacy path without
//! any graph_register calls.

use async_trait::async_trait;
use kyma_catalog::PostgresCatalog;
use kyma_connectors::catalog_sql;
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::runner::{ConnectorRunner, GraphRegisterFn, RowSink};
use kyma_connectors::scheduler::ConnectorScheduler;
use kyma_connectors::secrets::EnvSecretStore;
use kyma_connectors::{
    ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun, GraphHint, TableRows,
};
use kyma_core::catalog::{Catalog, NodeInfo, NodeRole};
use kyma_core::tenant::DEFAULT_TENANT;
use std::sync::{Arc, Mutex};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Recorded sink invocation.
#[derive(Clone, Debug)]
struct SinkCall {
    db: String,
    table: String,
    row_count: usize,
    idem_key: Option<String>,
}

/// A fake connector that emits two tables + a GraphHint on every tick.
struct TwoTableConn;

#[async_trait]
impl Connector for TwoTableConn {
    fn type_id(&self) -> &'static str {
        "two_table"
    }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }
    async fn run_once(
        &self,
        _ctx: &ConnectorCtx,
        _cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        Ok(ConnectorRun {
            rows: vec![],
            new_cursor: None,
            tables: vec![
                TableRows {
                    table: "t_nodes".into(),
                    rows: vec![
                        serde_json::json!({"id": "n1", "labels": "User"}),
                        serde_json::json!({"id": "n2", "labels": "Repo"}),
                    ],
                },
                TableRows {
                    table: "t_edges".into(),
                    rows: vec![serde_json::json!({"src": "n1", "dst": "n2", "type": "owns"})],
                },
            ],
            graph: Some(GraphHint {
                graph_name: "test_graph".into(),
                node_table: "t_nodes".into(),
                edge_table: "t_edges".into(),
            }),
        })
    }
}

/// A fake connector that uses only the legacy `rows` path (no `tables`, no
/// `graph`).
struct LegacyConn;

#[async_trait]
impl Connector for LegacyConn {
    fn type_id(&self) -> &'static str {
        "legacy"
    }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }
    async fn run_once(
        &self,
        _ctx: &ConnectorCtx,
        _cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        Ok(ConnectorRun {
            rows: vec![serde_json::json!({"metric": "cpu", "value": 0.5})],
            new_cursor: None,
            tables: vec![],
            graph: None,
        })
    }
}

/// Build a recording RowSink. Returns `(sink, calls_handle)` where
/// `calls_handle` is a shared `Vec<SinkCall>` you can inspect after the
/// run.
fn recording_sink() -> (RowSink, Arc<Mutex<Vec<SinkCall>>>) {
    let calls: Arc<Mutex<Vec<SinkCall>>> = Arc::new(Mutex::new(vec![]));
    let calls_clone = calls.clone();
    let sink: RowSink = Arc::new(move |db, table, rows, idem| {
        let record = SinkCall {
            db,
            table,
            row_count: rows.len(),
            idem_key: idem,
        };
        calls_clone.lock().unwrap().push(record);
        Box::pin(async move { Ok(()) })
    });
    (sink, calls)
}

/// Build a recording GraphRegisterFn. Returns `(fn, calls_handle)`.
fn recording_graph_register(
) -> (GraphRegisterFn, Arc<Mutex<Vec<(String, GraphHint)>>>) {
    let calls: Arc<Mutex<Vec<(String, GraphHint)>>> = Arc::new(Mutex::new(vec![]));
    let calls_clone = calls.clone();
    let f: GraphRegisterFn = Arc::new(move |db, hint| {
        calls_clone.lock().unwrap().push((db, hint));
        Box::pin(async move { Ok(()) })
    });
    (f, calls)
}

/// Spin up Postgres, register a connector that emits two tables + a
/// GraphHint, run one tick, and assert:
///  - The recording sink was called exactly TWICE (once per table).
///  - The two idempotency keys are DISTINCT (the critical invariant).
///  - The graph_register closure was called exactly ONCE with the right hint.
#[tokio::test]
async fn multi_table_distinct_idempotency_keys() {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());

    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(TwoTableConn));

    let _conn_id = catalog_sql::create_connector_direct(
        catalog.pool(),
        DEFAULT_TENANT,
        "two_table_test",
        "two_table",
        "graph_db",
        // target_table is empty — this connector uses the tables path.
        "",
        serde_json::json!({}),
        100,
        "periodic",
    )
    .await
    .unwrap();

    let sched = ConnectorScheduler::new(catalog.clone());
    sched.tick_once().await.unwrap();

    let lease = catalog
        .register_node(NodeInfo {
            role: NodeRole::Ingest,
            endpoint: "multi-table-test".into(),
            capabilities: serde_json::json!({}),
        })
        .await
        .unwrap();

    let (sink, sink_calls) = recording_sink();
    let (graph_fn, graph_calls) = recording_graph_register();

    let runner = ConnectorRunner::new(
        catalog.clone(),
        Arc::new(reg),
        sink,
        EnvSecretStore,
        lease.node_id,
    )
    .with_graph_register(graph_fn);

    runner.claim_and_run_one().await.expect("tick ran");

    // ---- Assertions on the sink ----
    let calls = sink_calls.lock().unwrap();
    assert_eq!(
        calls.len(),
        2,
        "expected sink called once per table, got {calls:?}"
    );

    // Find the two calls by table name.
    let nodes_call = calls.iter().find(|c| c.table == "t_nodes").expect("nodes call");
    let edges_call = calls.iter().find(|c| c.table == "t_edges").expect("edges call");

    assert_eq!(nodes_call.row_count, 2, "nodes row count");
    assert_eq!(edges_call.row_count, 1, "edges row count");
    assert_eq!(nodes_call.db, "graph_db");
    assert_eq!(edges_call.db, "graph_db");

    // THE CRITICAL INVARIANT: per-table idempotency keys must be distinct.
    let nodes_key = nodes_call.idem_key.as_deref().expect("nodes idem key");
    let edges_key = edges_call.idem_key.as_deref().expect("edges idem key");
    assert_ne!(
        nodes_key, edges_key,
        "idempotency keys for t_nodes and t_edges must be DISTINCT; \
         got nodes={nodes_key:?} edges={edges_key:?}"
    );

    // Both keys should contain the table name as a distinguishing component.
    assert!(
        nodes_key.contains("t_nodes"),
        "nodes key should contain table name: {nodes_key}"
    );
    assert!(
        edges_key.contains("t_edges"),
        "edges key should contain table name: {edges_key}"
    );

    // ---- Assertions on graph_register ----
    let graph = graph_calls.lock().unwrap();
    assert_eq!(graph.len(), 1, "graph_register must be called exactly once");
    let (db, hint) = &graph[0];
    assert_eq!(db, "graph_db");
    assert_eq!(hint.graph_name, "test_graph");
    assert_eq!(hint.node_table, "t_nodes");
    assert_eq!(hint.edge_table, "t_edges");
}

/// Verify the legacy `rows` path still works as-before: the sink is called
/// exactly once, with the connector's target_table, and graph_register is
/// NOT called.
#[tokio::test]
async fn legacy_rows_path_unchanged() {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());

    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(LegacyConn));

    catalog_sql::create_connector_direct(
        catalog.pool(),
        DEFAULT_TENANT,
        "legacy_test",
        "legacy",
        "metrics_db",
        "prom_metrics",
        serde_json::json!({}),
        100,
        "periodic",
    )
    .await
    .unwrap();

    let sched = ConnectorScheduler::new(catalog.clone());
    sched.tick_once().await.unwrap();

    let lease = catalog
        .register_node(NodeInfo {
            role: NodeRole::Ingest,
            endpoint: "legacy-test".into(),
            capabilities: serde_json::json!({}),
        })
        .await
        .unwrap();

    let (sink, sink_calls) = recording_sink();
    let (graph_fn, graph_calls) = recording_graph_register();

    let runner = ConnectorRunner::new(
        catalog.clone(),
        Arc::new(reg),
        sink,
        EnvSecretStore,
        lease.node_id,
    )
    .with_graph_register(graph_fn);

    runner.claim_and_run_one().await.expect("tick ran");

    let calls = sink_calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "legacy path: sink called exactly once");
    assert_eq!(calls[0].table, "prom_metrics");
    assert_eq!(calls[0].db, "metrics_db");
    assert_eq!(calls[0].row_count, 1);

    let graph = graph_calls.lock().unwrap();
    assert_eq!(
        graph.len(),
        0,
        "legacy path must NOT trigger graph_register"
    );
}
