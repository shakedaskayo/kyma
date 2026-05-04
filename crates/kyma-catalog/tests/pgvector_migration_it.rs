//! Confirms migration 004 runs, pgvector loads, the HNSW index exists,
//! and the partial-unique indexes enforce one row per (kind, scope).

#![cfg(test)]

use kyma_core::tenant::DEFAULT_TENANT;
use sqlx::{Pool, Postgres};
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres as PgContainer;

/// Spin up a pgvector-enabled Postgres and apply all migrations.
async fn start_pg() -> (ContainerAsync<PgContainer>, Pool<Postgres>) {
    // pgvector/pgvector:pg16 includes the vector extension out of the box.
    let container = PgContainer::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    (container, pool)
}

#[tokio::test]
async fn pgvector_extension_loaded_and_hnsw_index_present() {
    let (_c, pool) = start_pg().await;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'vector')")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(exists, "pgvector extension not loaded");

    let idx: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_indexes
                       WHERE indexname = 'schema_embeddings_hnsw')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(idx, "HNSW index missing");
}

#[tokio::test]
async fn schema_embeddings_partial_uniques_enforce_one_row_per_kind() {
    let (_c, pool) = start_pg().await;
    let zero_vec: Vec<f32> = vec![0.0; 384];
    let v: pgvector::Vector = zero_vec.into();

    // First 'table' row succeeds.
    sqlx::query(
        "INSERT INTO schema_embeddings
        (tenant_id, database, table_name, kind, text_source, text_source_sha256,
         model_id, embedding)
        VALUES ($1, $2, $3, 'table', 'a', '\\x00', 'm', $4)",
    )
    .bind(DEFAULT_TENANT.as_uuid())
    .bind("db")
    .bind("t")
    .bind(&v)
    .execute(&pool)
    .await
    .unwrap();

    // Duplicate ('table' kind with the same db/table/model, column_name IS NULL)
    // must be rejected by the partial-unique index.
    let dup = sqlx::query(
        "INSERT INTO schema_embeddings
        (tenant_id, database, table_name, kind, text_source, text_source_sha256,
         model_id, embedding)
        VALUES ($1, $2, $3, 'table', 'a', '\\x00', 'm', $4)",
    )
    .bind(DEFAULT_TENANT.as_uuid())
    .bind("db")
    .bind("t")
    .bind(&v)
    .execute(&pool)
    .await;
    assert!(dup.is_err(), "second 'table' row should have been rejected");

    // A 'column' row for the same (db, table, model) is allowed because
    // it uses the other partial-unique index (column_name IS NOT NULL).
    sqlx::query(
        "INSERT INTO schema_embeddings
        (tenant_id, database, table_name, column_name, kind, text_source, text_source_sha256,
         model_id, embedding)
        VALUES ($1, $2, $3, $4, 'column', 'b', '\\x01', 'm', $5)",
    )
    .bind(DEFAULT_TENANT.as_uuid())
    .bind("db")
    .bind("t")
    .bind("col_a")
    .bind(&v)
    .execute(&pool)
    .await
    .unwrap();

    // Duplicate column row rejected.
    let dup_col = sqlx::query(
        "INSERT INTO schema_embeddings
        (tenant_id, database, table_name, column_name, kind, text_source, text_source_sha256,
         model_id, embedding)
        VALUES ($1, $2, $3, $4, 'column', 'b', '\\x01', 'm', $5)",
    )
    .bind(DEFAULT_TENANT.as_uuid())
    .bind("db")
    .bind("t")
    .bind("col_a")
    .bind(&v)
    .execute(&pool)
    .await;
    assert!(
        dup_col.is_err(),
        "second column row should have been rejected"
    );
}
