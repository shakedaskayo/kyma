//! Integration test — scheduler + runner + a fake DataSource.

use async_trait::async_trait;
use pensieve_catalog::PostgresCatalog;
use pensieve_datasources::catalog_sql;
use pensieve_datasources::registry::DataSourceRegistry;
use pensieve_datasources::runner::DataSourceRunner;
use pensieve_datasources::secrets::EnvSecretStore;
use pensieve_datasources::{ConfigError, DataSource, DataSourceCtx, DataSourceError, DataSourceRun};
use pensieve_core::catalog::{Catalog, NodeInfo, NodeRole};
use pensieve_core::tenant::DEFAULT_TENANT;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[derive(Default)]
struct CountingConn {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl DataSource for CountingConn {
    fn type_id(&self) -> &'static str {
        "counter"
    }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }
    async fn run_once(
        &self,
        _ctx: &DataSourceCtx,
        _cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<DataSourceRun, DataSourceError> {
        let n = cursor.and_then(|v| v.as_u64()).unwrap_or(0);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(DataSourceRun {
            rows: vec![serde_json::json!({"n": n, "ok": true})],
            new_cursor: Some(serde_json::json!(n + 1)),
            tables: vec![],
            graph: None,
        })
    }
}

#[tokio::test]
async fn runner_claims_and_updates_cursor() {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());

    let calls = Arc::new(AtomicUsize::new(0));
    let fake = Arc::new(CountingConn {
        calls: calls.clone(),
    });
    let mut reg = DataSourceRegistry::new();
    reg.register(fake.clone());

    let id = catalog_sql::create_data_source_direct(
        catalog.pool(),
        DEFAULT_TENANT,
        "c1",
        "counter",
        "db",
        "tbl",
        serde_json::json!({}),
        100,
        "periodic",
    )
    .await
    .unwrap();

    // Enqueue directly on the legacy background_tasks harness this test
    // drives; the production scheduler now enqueues fabric `datasource_sync`
    // jobs (covered by pensieve-jobs' integration test).
    catalog_sql::enqueue_tick(catalog.pool(), DEFAULT_TENANT, id, 1_000)
        .await
        .unwrap();

    // Register a node so we have a NodeId to hand to the runner.
    let lease = catalog
        .register_node(NodeInfo {
            role: NodeRole::Ingest,
            endpoint: "data-source-runner:test".into(),
            capabilities: serde_json::json!({"data_source_runner": true}),
        })
        .await
        .unwrap();

    // Runner uses a stubbed RowSink — closes over a counter. Avoids
    // depending on a live WritePath here. Real WritePath wiring is
    // covered by the E2E test script.
    let sink: pensieve_datasources::runner::RowSink =
        Arc::new(|_db, _tbl, _rows, _idem| Box::pin(async move { Ok(()) }));
    let runner = DataSourceRunner::new(
        catalog.clone(),
        Arc::new(reg),
        sink,
        EnvSecretStore,
        lease.node_id,
    );
    runner.claim_and_run_one().await.expect("tick ran");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let cur = catalog_sql::load_cursor(catalog.pool(), DEFAULT_TENANT, id)
        .await
        .unwrap();
    assert_eq!(cur, Some(serde_json::json!(1)));
}
