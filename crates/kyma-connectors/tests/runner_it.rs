//! Integration test — scheduler + runner + a fake Connector.

use async_trait::async_trait;
use kyma_catalog::PostgresCatalog;
use kyma_connectors::catalog_sql;
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::runner::ConnectorRunner;
use kyma_connectors::scheduler::ConnectorScheduler;
use kyma_connectors::secrets::EnvSecretStore;
use kyma_connectors::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

#[derive(Default)]
struct CountingConn {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Connector for CountingConn {
    fn type_id(&self) -> &'static str {
        "counter"
    }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> {
        Ok(())
    }
    async fn run_once(
        &self,
        _ctx: &ConnectorCtx,
        _cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        let n = cursor.and_then(|v| v.as_u64()).unwrap_or(0);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ConnectorRun {
            rows: vec![serde_json::json!({"n": n, "ok": true})],
            new_cursor: Some(serde_json::json!(n + 1)),
        })
    }
}

#[tokio::test]
async fn runner_claims_and_updates_cursor() {
    let pg = Postgres::default().start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());

    let calls = Arc::new(AtomicUsize::new(0));
    let fake = Arc::new(CountingConn {
        calls: calls.clone(),
    });
    let mut reg = ConnectorRegistry::new();
    reg.register(fake.clone());

    let id = catalog_sql::create_connector_direct(
        catalog.pool(),
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

    let sched = ConnectorScheduler::new(catalog.clone());
    sched.tick_once().await.unwrap();

    // Runner uses a stubbed RowSink — closes over a counter. Avoids
    // depending on a live WritePath here. Real WritePath wiring is
    // covered by the E2E test script.
    let sink: kyma_connectors::runner::RowSink =
        Arc::new(|_db, _tbl, _rows, _idem| Box::pin(async move { Ok(()) }));
    let runner = ConnectorRunner::new(
        catalog.clone(),
        Arc::new(reg),
        sink,
        EnvSecretStore,
        "node-a".into(),
    );
    runner.claim_and_run_one().await.expect("tick ran");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let cur = catalog_sql::load_cursor(catalog.pool(), id).await.unwrap();
    assert_eq!(cur, Some(serde_json::json!(1)));
}
