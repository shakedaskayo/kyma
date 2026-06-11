//! Verifies 027 renames the connector tables in place: rows seeded under the
//! old names survive the rename, queued ticks/jobs are rekeyed, capability
//! arrays are rewritten, and the watcher registry table appears.

use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

const TENANT: &str = "00000000-0000-0000-0000-000000000000";
const DS_ID: &str = "11111111-1111-1111-1111-111111111111";

#[tokio::test]
async fn rename_migration_preserves_rows() {
    let pg = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = PgPoolOptions::new().connect(&url).await.unwrap();

    let mut files: Vec<_> = std::fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kyma-catalog/migrations"
    ))
    .unwrap()
    .map(|e| e.unwrap().path())
    .filter(|p| p.extension().is_some_and(|e| e == "sql"))
    .collect();
    files.sort();
    let split = files
        .iter()
        .position(|p| {
            p.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("027_")
        })
        .expect("027 migration present");

    for f in &files[..split] {
        let sql = std::fs::read_to_string(f).unwrap();
        sqlx::raw_sql(&sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{f:?}: {e}"));
    }

    // Seed under the OLD names, matching the real schema as of 026
    // (005 base tables + 007 tenant columns + 017 oauth_flows +
    // 022 workers/jobs + 026 artifacts).
    let seed = format!(
        r#"
        INSERT INTO connectors
            (id, tenant_id, name, type, target_database, target_table,
             config_jsonb, schedule_ms, drive_model)
        VALUES
            ('{DS_ID}', '{TENANT}', 'old-one', 'github', 'db', 'events',
             '{{"repo":"o/r"}}'::jsonb, 60000, 'periodic');

        INSERT INTO connector_cursors (connector_id, tenant_id, cursor_jsonb)
        VALUES ('{DS_ID}', '{TENANT}', '{{"page":2}}'::jsonb);

        INSERT INTO connector_leases (connector_id, tenant_id, node_id, expires_at)
        VALUES ('{DS_ID}', '{TENANT}', 'node-1', now() + interval '1 hour');

        INSERT INTO background_tasks (tenant_id, kind, payload, priority)
        VALUES ('{TENANT}', 'connector_tick',
                jsonb_build_object('tenant_id', '{TENANT}'::text,
                                   'connector_id', '{DS_ID}'::text,
                                   'scheduled_for', '60000'),
                0);

        INSERT INTO background_tasks (tenant_id, kind, payload, priority)
        VALUES ('{TENANT}', 'connector_sync',
                jsonb_build_object('tenant_id', '{TENANT}'::text,
                                   'connector_id', '{DS_ID}'::text,
                                   'scheduled_for', '60000'),
                0);

        INSERT INTO workers (tenant_id, name, kind, capabilities)
        VALUES ('{TENANT}', 'embedded@test', 'embedded',
                ARRAY['connector','dreaming','llm']);

        INSERT INTO jobs (tenant_id, kind, payload, req_capabilities)
        VALUES ('{TENANT}', 'connector_sync',
                jsonb_build_object('tenant_id', '{TENANT}'::text,
                                   'connector_id', '{DS_ID}'::text,
                                   'scheduled_for', '60000'),
                ARRAY['connector']);

        INSERT INTO oauth_flows
            (tenant_id, state, provider, connector_type, label, redirect_uri, expires_at)
        VALUES ('{TENANT}', 'state-1', 'github', 'github', 'lbl',
                'http://localhost/cb', now() + interval '10 minutes');

        INSERT INTO artifacts (tenant_id, object_path, source, connector_id)
        VALUES ('{TENANT}', 'logs/run-1', 'github', '{DS_ID}');

        WITH d AS (
            INSERT INTO databases (tenant_id, name)
            VALUES ('{TENANT}', 'mig-test-db') RETURNING id
        ), t AS (
            INSERT INTO tables (tenant_id, database_id, name)
            SELECT '{TENANT}', id, 'events' FROM d RETURNING id
        ), ss AS (
            INSERT INTO schema_snapshots (tenant_id, table_id, arrow_schema)
            SELECT '{TENANT}', id, '{{}}'::jsonb FROM t RETURNING id
        ), s AS (
            INSERT INTO snapshots (tenant_id, table_id, sequence_number, schema_snapshot_id)
            SELECT '{TENANT}', t.id, 1, ss.id FROM t, ss RETURNING id
        )
        INSERT INTO ingest_ledger
            (tenant_id, idempotency_key, table_id, snapshot_id, rows_ingested,
             bytes_written, ttl_expires_at)
        SELECT '{TENANT}', 'connector:{DS_ID}:60000000000', t.id, s.id, 1, 1,
               now() + interval '1 day'
        FROM t, s;
        "#
    );
    sqlx::raw_sql(&seed).execute(&pool).await.unwrap();

    for f in &files[split..] {
        let sql = std::fs::read_to_string(f).unwrap();
        sqlx::raw_sql(&sql)
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{f:?}: {e}"));
    }

    // data_sources row survived the table rename.
    let name: String =
        sqlx::query_scalar("SELECT name FROM data_sources WHERE id = $1::uuid")
            .bind(DS_ID)
            .fetch_one(&pool)
            .await
            .expect("data_sources row");
    assert_eq!(name, "old-one");

    // Cursor + lease rows survived (table + column rename).
    let page: i64 = sqlx::query_scalar(
        "SELECT (cursor_jsonb->>'page')::bigint FROM data_source_cursors
         WHERE data_source_id = $1::uuid",
    )
    .bind(DS_ID)
    .fetch_one(&pool)
    .await
    .expect("data_source_cursors row");
    assert_eq!(page, 2);
    let lease_node: String = sqlx::query_scalar(
        "SELECT node_id FROM data_source_leases WHERE data_source_id = $1::uuid",
    )
    .bind(DS_ID)
    .fetch_one(&pool)
    .await
    .expect("data_source_leases row");
    assert_eq!(lease_node, "node-1");

    // Queued background tick was rekeyed + kind-flipped; nothing left behind
    // under the old vocabulary.
    let (n_new,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM background_tasks
         WHERE kind = 'data_source_tick'
           AND payload ? 'data_source_id'
           AND payload->>'data_source_id' = $1",
    )
    .bind(DS_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n_new, 1, "rekeyed data_source_tick present");
    let (n_sync,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM background_tasks
         WHERE kind = 'data_source_sync'
           AND payload ? 'data_source_id'
           AND payload->>'data_source_id' = $1",
    )
    .bind(DS_ID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n_sync, 1, "rekeyed data_source_sync present");
    let (n_old,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM background_tasks
         WHERE kind IN ('connector_tick','connector_sync') OR payload ? 'connector_id'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n_old, 0, "no connector-keyed background_tasks remain");

    // Fabric job rekeyed the same way, including the capability array.
    let job = sqlx::query(
        "SELECT payload, req_capabilities FROM jobs WHERE kind = 'data_source_sync'",
    )
    .fetch_one(&pool)
    .await
    .expect("data_source_sync job");
    let payload: serde_json::Value = job.get("payload");
    assert_eq!(payload["data_source_id"], DS_ID);
    assert!(payload.get("connector_id").is_none());
    let caps: Vec<String> = job.get("req_capabilities");
    assert_eq!(caps, vec!["data_source".to_string()]);
    let (n_old_jobs,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM jobs
         WHERE kind = 'connector_sync' OR payload ? 'connector_id'
            OR 'connector' = ANY(req_capabilities)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n_old_jobs, 0);

    // Worker capability vocabulary rewritten in place.
    let worker_caps: Vec<String> =
        sqlx::query_scalar("SELECT capabilities FROM workers WHERE name = 'embedded@test'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(worker_caps, vec!["data_source", "dreaming", "llm"]);

    // oauth_flows.connector_type became data_source_type.
    let ds_type: String =
        sqlx::query_scalar("SELECT data_source_type FROM oauth_flows WHERE state = 'state-1'")
            .fetch_one(&pool)
            .await
            .expect("oauth_flows.data_source_type");
    assert_eq!(ds_type, "github");

    // artifacts.connector_id became data_source_id.
    let art_ds: uuid::Uuid = sqlx::query_scalar(
        "SELECT data_source_id FROM artifacts WHERE object_path = 'logs/run-1'",
    )
    .fetch_one(&pool)
    .await
    .expect("artifacts.data_source_id");
    assert_eq!(art_ds.to_string(), DS_ID);

    // Ingest dedup keys carry the new prefix.
    let (n_keys,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM ingest_ledger WHERE idempotency_key LIKE 'data_source:%'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n_keys, 1);

    // Dedup indexes were rebuilt under the new names/expressions.
    let (idx,): (String,) = sqlx::query_as(
        "SELECT indexdef FROM pg_indexes
         WHERE tablename = 'background_tasks'
           AND indexname = 'background_tasks_data_source_tick_uniq'",
    )
    .fetch_one(&pool)
    .await
    .expect("new background_tasks dedup index");
    assert!(idx.contains("data_source_tick") && idx.contains("data_source_id"));
    let (jdx,): (String,) = sqlx::query_as(
        "SELECT indexdef FROM pg_indexes
         WHERE tablename = 'jobs' AND indexname = 'jobs_data_source_sync_uniq'",
    )
    .fetch_one(&pool)
    .await
    .expect("new jobs dedup index");
    assert!(jdx.contains("data_source_sync") && jdx.contains("data_source_id"));
    let (n_old_idx,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM pg_indexes
         WHERE indexname IN ('background_tasks_connector_tick_uniq','jobs_connector_sync_uniq')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n_old_idx, 0);

    // Watcher registry exists and starts empty.
    let (n_watchers,): (i64,) = sqlx::query_as("SELECT count(*) FROM data_source_watchers")
        .fetch_one(&pool)
        .await
        .expect("data_source_watchers table");
    assert_eq!(n_watchers, 0);
}
