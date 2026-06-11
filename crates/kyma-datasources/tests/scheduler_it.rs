//! Integration test (testcontainers) for the scheduler.

use kyma_catalog::PostgresCatalog;
use kyma_datasources::catalog_sql;
use kyma_datasources::scheduler::DataSourceScheduler;
use kyma_core::tenant::DEFAULT_TENANT;
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn pg_catalog() -> (
    testcontainers::ContainerAsync<Postgres>,
    Arc<PostgresCatalog>,
) {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());
    (pg, catalog)
}

#[tokio::test]
async fn inserts_tick_after_interval() {
    let (_pg, catalog) = pg_catalog().await;
    let id = catalog_sql::create_data_source_direct(
        catalog.pool(),
        DEFAULT_TENANT,
        "p1",
        "prometheus",
        "db",
        "metrics",
        serde_json::json!({ "endpoint": "http://x/metrics" }),
        100,
        "periodic",
    )
    .await
    .unwrap();

    let sched = DataSourceScheduler::new(catalog.clone());
    sched.tick_once().await.expect("first tick");
    sched
        .tick_once()
        .await
        .expect("second tick is a no-op (dedup)");

    // Since the datasource_sync cutover, scheduler ticks land on the worker
    // fabric's `jobs` queue (not `background_tasks`).
    let rows = sqlx::query_as::<_, (i64,)>(
        "SELECT count(*) FROM jobs
         WHERE kind = 'data_source_sync'
           AND payload->>'data_source_id' = $1::text",
    )
    .bind(id.to_string())
    .fetch_one(catalog.pool())
    .await
    .unwrap();
    assert_eq!(rows.0, 1, "exactly one task enqueued");
}

#[tokio::test]
async fn disabled_data_sources_are_skipped() {
    let (_pg, catalog) = pg_catalog().await;
    let id = catalog_sql::create_data_source_direct(
        catalog.pool(),
        DEFAULT_TENANT,
        "p2",
        "prometheus",
        "db",
        "metrics",
        serde_json::json!({ "endpoint": "http://x/metrics" }),
        100,
        "periodic",
    )
    .await
    .unwrap();
    sqlx::query("UPDATE data_sources SET enabled = FALSE WHERE id = $1")
        .bind(id)
        .execute(catalog.pool())
        .await
        .unwrap();

    let sched = DataSourceScheduler::new(catalog.clone());
    sched.tick_once().await.unwrap();

    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM background_tasks WHERE kind = 'data_source_tick'")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
}
