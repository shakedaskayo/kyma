//! Smoke test: migration 005 applies cleanly and creates expected objects.

use kyma_catalog::PostgresCatalog;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn migration_005_creates_connector_tables() {
    let pg = Postgres::default().start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("connect + migrate");

    let pool = catalog.pool();
    sqlx::query("SELECT 1 FROM connectors LIMIT 0")
        .execute(pool)
        .await
        .expect("connectors table exists");
    sqlx::query("SELECT 1 FROM connector_cursors LIMIT 0")
        .execute(pool)
        .await
        .expect("connector_cursors table exists");
    sqlx::query("SELECT 1 FROM connector_leases LIMIT 0")
        .execute(pool)
        .await
        .expect("connector_leases table exists");

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_indexes
         WHERE tablename = 'background_tasks'
         AND indexname = 'background_tasks_connector_tick_uniq'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_indexes");
    assert_eq!(count, 1, "dedup index should be present");

    let (indexdef,): (String,) = sqlx::query_as(
        "SELECT indexdef FROM pg_indexes
         WHERE tablename = 'background_tasks'
         AND indexname = 'background_tasks_connector_tick_uniq'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_indexes indexdef");
    assert!(
        indexdef.contains("UNIQUE INDEX"),
        "dedup index must be UNIQUE, got: {indexdef}"
    );
    assert!(
        indexdef.contains("connector_tick"),
        "dedup index must be partial on kind='connector_tick', got: {indexdef}"
    );
    assert!(
        indexdef.contains("pending") && indexdef.contains("claimed"),
        "dedup index predicate must include pending/claimed, got: {indexdef}"
    );
}
