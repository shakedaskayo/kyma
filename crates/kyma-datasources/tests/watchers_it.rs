//! Integration tests for the watcher registry (server-mode watcher
//! provenance): register (upsert), heartbeat with scan stats, list with
//! staleness computation, and 24h prune.

use kyma_catalog::PostgresCatalog;
use kyma_datasources::watchers::{ScanStats, WatcherRegistry};
use serde_json::json;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn pg_pool() -> (testcontainers::ContainerAsync<Postgres>, sqlx::PgPool) {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    // PostgresCatalog::connect runs migrations (incl. data_source_watchers).
    let catalog = PostgresCatalog::connect(&url).await.unwrap();
    let pool = catalog.pool().clone();
    (pg, pool)
}

#[tokio::test]
async fn register_heartbeat_list_prune() {
    let (_pg, pool) = pg_pool().await;

    let config = json!({"prefixes": ["drops/"], "poll_secs": 5});

    // 1. register
    let reg = WatcherRegistry::register(
        pool.clone(),
        "filedrop",
        "host-a",
        "node-a",
        "shaked",
        config.clone(),
    )
    .await
    .unwrap();

    // 2. register again with identical args -> same id (upsert)
    let reg2 = WatcherRegistry::register(
        pool.clone(),
        "filedrop",
        "host-a",
        "node-a",
        "shaked",
        config.clone(),
    )
    .await
    .unwrap();
    assert_eq!(reg.id(), reg2.id(), "identical registration must upsert to the same row");

    // 3. heartbeat with scan stats (best-effort: no Result to unwrap)
    reg.heartbeat(Some(&ScanStats {
        seen: 10,
        processed: 9,
        errors: 1,
        duration_ms: 120,
        at: "2026-06-11T00:00:00Z".into(),
        detail: None,
    }))
    .await;

    // 4. list -> one row, fresh
    let rows = WatcherRegistry::list(&pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.id, reg.id());
    assert_eq!(row.kind, "filedrop");
    assert_eq!(row.node_host, "host-a");
    assert_eq!(row.node_id, "node-a");
    assert_eq!(row.identity, "shaked");
    assert_eq!(row.config["poll_secs"], 5);
    let scan = row.last_scan.as_ref().expect("heartbeat stored scan stats");
    assert_eq!(scan["processed"], 9);
    assert_eq!(scan["seen"], 10);
    assert!(!row.stale, "fresh heartbeat must not be stale");
    // timestamps round-trip as RFC 3339
    assert!(
        chrono::DateTime::parse_from_rfc3339(&row.last_heartbeat_at).is_ok(),
        "last_heartbeat_at not RFC 3339: {}",
        row.last_heartbeat_at
    );
    assert!(
        chrono::DateTime::parse_from_rfc3339(&row.started_at).is_ok(),
        "started_at not RFC 3339: {}",
        row.started_at
    );

    // 5. age the heartbeat past 3x poll_secs (15s) -> stale
    sqlx::query(
        "UPDATE data_source_watchers SET last_heartbeat_at = now() - interval '20 seconds' WHERE id = $1",
    )
    .bind(reg.id())
    .execute(&pool)
    .await
    .unwrap();
    let rows = WatcherRegistry::list(&pool).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].stale, "heartbeat older than 3x poll_secs must be stale");

    // 6. age past 24h -> pruned by list()
    sqlx::query(
        "UPDATE data_source_watchers SET last_heartbeat_at = now() - interval '25 hours' WHERE id = $1",
    )
    .bind(reg.id())
    .execute(&pool)
    .await
    .unwrap();
    let rows = WatcherRegistry::list(&pool).await.unwrap();
    assert!(rows.is_empty(), "watchers silent for >24h must be pruned");
}
