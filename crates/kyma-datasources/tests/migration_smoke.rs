//! Smoke test: migrations apply cleanly and creates expected objects.

use kyma_catalog::PostgresCatalog;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn migrations_create_data_source_tables() {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("connect + migrate");

    let pool = catalog.pool();
    sqlx::query("SELECT 1 FROM data_sources LIMIT 0")
        .execute(pool)
        .await
        .expect("data_sources table exists");
    sqlx::query("SELECT 1 FROM data_source_cursors LIMIT 0")
        .execute(pool)
        .await
        .expect("data_source_cursors table exists");
    sqlx::query("SELECT 1 FROM data_source_leases LIMIT 0")
        .execute(pool)
        .await
        .expect("data_source_leases table exists");

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_indexes
         WHERE tablename = 'background_tasks'
         AND indexname = 'background_tasks_data_source_tick_uniq'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_indexes");
    assert_eq!(count, 1, "dedup index should be present");

    let (indexdef,): (String,) = sqlx::query_as(
        "SELECT indexdef FROM pg_indexes
         WHERE tablename = 'background_tasks'
         AND indexname = 'background_tasks_data_source_tick_uniq'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_indexes indexdef");
    assert!(
        indexdef.contains("UNIQUE INDEX"),
        "dedup index must be UNIQUE, got: {indexdef}"
    );
    assert!(
        indexdef.contains("data_source_tick"),
        "dedup index must be partial on kind='data_source_tick', got: {indexdef}"
    );
    assert!(
        indexdef.contains("pending") && indexdef.contains("claimed"),
        "dedup index predicate must include pending/claimed, got: {indexdef}"
    );
}
