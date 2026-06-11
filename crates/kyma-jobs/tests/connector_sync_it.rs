//! Integration test — the production connector path on the worker fabric:
//! scheduler enqueues a `connector_sync` job, the embedded worker claims it
//! via [`PgQueue`], the [`ConnectorSyncExecutor`] runs the tick body, and the
//! cursor + job status advance.

use async_trait::async_trait;
use kyma_catalog::{PgFabricStore, PostgresCatalog};
use kyma_datasources::catalog_sql;
use kyma_datasources::registry::ConnectorRegistry;
use kyma_datasources::runner::{ConnectorTickDeps, PgConnectorControl, RowSink};
use kyma_datasources::scheduler::ConnectorScheduler;
use kyma_datasources::secrets::EnvSecretStore;
use kyma_datasources::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};
use kyma_core::fabric::{WorkerKind, WorkerRegistration};
use kyma_core::tenant::DEFAULT_TENANT;
use kyma_jobs::connector_sync::ConnectorSyncExecutor;
use kyma_jobs::{ExecutorRegistry, JobRunner, PgQueue};
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
            tables: vec![],
            graph: None,
        })
    }
}

#[tokio::test]
async fn scheduler_enqueues_fabric_job_and_worker_runs_it() {
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
    let mut reg = ConnectorRegistry::new();
    reg.register(fake.clone());

    let conn_id = catalog_sql::create_connector_direct(
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

    // The production scheduler enqueues a fabric `connector_sync` job.
    let sched = ConnectorScheduler::new(catalog.clone());
    sched.tick_once().await.unwrap();
    // Idempotent: a second tick in the same bucket inserts nothing.
    sched.tick_once().await.unwrap();
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM jobs WHERE kind = 'connector_sync'")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
    assert_eq!(pending, 1, "scheduler enqueue must be idempotent per bucket");

    // Embedded worker registers in the fabric and claims the job.
    let fabric = Arc::new(PgFabricStore::new(catalog.pool().clone()));
    let caps: Vec<String> = vec!["connector".into()];
    let worker_id = fabric
        .upsert_embedded_worker(
            DEFAULT_TENANT,
            &WorkerRegistration {
                name: "embedded@test".into(),
                kind: WorkerKind::Embedded,
                hostname: None,
                capabilities: caps.clone(),
                labels: serde_json::json!({}),
                sources: serde_json::json!({}),
                max_concurrent: 1,
                version: None,
            },
        )
        .await
        .unwrap();

    let sink: RowSink = Arc::new(|_db, _tbl, _rows, _idem| Box::pin(async move { Ok(()) }));
    let deps = ConnectorTickDeps {
        control: Arc::new(PgConnectorControl::new(catalog.pool().clone())),
        registry: Arc::new(reg),
        sink,
        graph_register: Arc::new(|_db, _hint| Box::pin(async move { Ok(()) })),
        secrets: Arc::new(EnvSecretStore),
        credentials: Arc::new(kyma_datasources::runner::NoopCredentialStore),
        oauth: None,
        artifacts: None,
        catalog: None,
    };
    let mut executors = ExecutorRegistry::new();
    executors.register(Arc::new(ConnectorSyncExecutor::new(deps)));
    let queue = Arc::new(PgQueue::new(fabric.clone(), worker_id, None, caps, 60));
    let runner = JobRunner::new(queue, executors, 60);

    let ran = runner
        .claim_and_run_one(&["connector_sync".to_string()])
        .await
        .unwrap();
    assert!(ran, "worker should have claimed the job");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Cursor advanced and the job is done with the row count in the result.
    let cur = catalog_sql::load_cursor(catalog.pool(), DEFAULT_TENANT, conn_id)
        .await
        .unwrap();
    assert_eq!(cur, Some(serde_json::json!(1)));
    let (status, result): (String, serde_json::Value) = sqlx::query_as(
        "SELECT status, result FROM jobs WHERE kind = 'connector_sync' LIMIT 1",
    )
    .fetch_one(catalog.pool())
    .await
    .unwrap();
    assert_eq!(status, "done");
    assert_eq!(result, serde_json::json!({"rows": 1}));
}
