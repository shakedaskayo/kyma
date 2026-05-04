//! Integration tests for `POST /v1/database/:db/table/:table/cleanup`.
//!
//! Requires `--features kyma-server/test-support` to compile.
//! Each test spins up its own isolated Postgres container via testcontainers.

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, TableConfig};
use kyma_core::tenant::DEFAULT_TENANT;
use serde_json::Value;
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use tower::ServiceExt;

// -------------------------------------------------------------------------
// Helper: build the cleanup write router with auth
// -------------------------------------------------------------------------

fn cleanup_write_app(
    catalog: Arc<dyn Catalog>,
) -> impl tower::Service<
    axum::http::Request<axum::body::Body>,
    Response = axum::http::Response<axum::body::Body>,
    Error = std::convert::Infallible,
    Future = impl std::future::Future<
        Output = Result<axum::http::Response<axum::body::Body>, std::convert::Infallible>,
    >,
> {
    let backend: std::sync::Arc<dyn kyma_server::auth::AuthBackend> = std::sync::Arc::new(
        kyma_server::auth::EnvAuthBackend::from_str("test-write-token:write"),
    );
    kyma_server::cleanup_write_router(catalog).layer(axum::middleware::from_fn_with_state(
        kyma_server::auth::AuthLayerState {
            backend,
            required: kyma_server::auth::Role::Write,
        },
        kyma_server::auth::require_role_middleware,
    ))
}

/// Send a POST cleanup request and return the response.
async fn post_cleanup<S>(
    app: S,
    db: &str,
    table: &str,
    before: &str,
) -> axum::http::Response<Body>
where
    S: tower::Service<
        Request<Body>,
        Response = axum::http::Response<Body>,
        Error = std::convert::Infallible,
    >,
{
    // Encode the `before` timestamp for a query string (replace `+` with `%2B`,
    // `:` with `%3A`). In practice our test values only have colons to encode.
    let before_enc = before.replace(':', "%3A").replace('+', "%2B");
    let uri = format!("/v1/database/{db}/table/{table}/cleanup?before={before_enc}");
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", "Bearer test-write-token")
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

/// Parse the response body as JSON.
async fn body_json(resp: axum::http::Response<Body>) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

// -------------------------------------------------------------------------
// Fixture helpers
// -------------------------------------------------------------------------

/// Spin up a pgvector-enabled Postgres container and return the connected raw
/// `PostgresCatalog` (not erased behind `dyn Catalog`) so tests can call
/// `pool()` for direct SQL seeding.
async fn raw_catalog() -> (PostgresCatalog, testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>) {
    let container = testcontainers_modules::postgres::Postgres::default()
        .with_user("kyma")
        .with_password("kyma_dev")
        .with_db_name("kyma")
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("testcontainers: failed to start Postgres");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("testcontainers: failed to get mapped port");
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");
    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("catalog connect + migrate");
    (catalog, container)
}

fn sample_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            false,
        ),
        Field::new("body", DataType::Utf8, true),
    ]))
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

/// Seeding a soft-deleted extent and calling cleanup returns the right counts
/// and removes the row.
#[tokio::test]
async fn cleanup_removes_soft_deleted_extents_and_returns_counts() {
    let (catalog, _container) = raw_catalog().await;

    // 1. Create database + table.
    let db_id = catalog
        .create_database("testdb")
        .await
        .expect("create_database");
    let table_id = catalog
        .create_table(db_id, "testtable", sample_schema(), TableConfig::default())
        .await
        .expect("create_table");

    // 2. Seed one soft-deleted extent (deleted 2 days ago) directly via SQL.
    let pool = catalog.pool();
    let table_uuid = *table_id.as_uuid();
    let schema_snap_uuid: uuid::Uuid = sqlx::query_scalar(
        "SELECT schema_snapshot_id FROM tables WHERE id = $1",
    )
    .bind(table_uuid)
    .fetch_one(pool)
    .await
    .expect("fetch schema_snapshot_id");

    // Insert a soft-deleted extent with known byte_size + row_count.
    sqlx::query(
        "INSERT INTO extents (tenant_id, table_id, schema_snapshot_id, object_path, byte_size, row_count, deleted_at)
         VALUES ($1, $2, $3, 'test/path/old.kyma', 4096, 200, now() - interval '2 days')",
    )
    .bind(DEFAULT_TENANT.as_uuid())
    .bind(table_uuid)
    .bind(schema_snap_uuid)
    .execute(pool)
    .await
    .expect("insert soft-deleted extent");

    // 3. Seed a second, live (not soft-deleted) extent — should NOT be cleaned.
    sqlx::query(
        "INSERT INTO extents (tenant_id, table_id, schema_snapshot_id, object_path, byte_size, row_count)
         VALUES ($1, $2, $3, 'test/path/live.kyma', 2048, 50)",
    )
    .bind(DEFAULT_TENANT.as_uuid())
    .bind(table_uuid)
    .bind(schema_snap_uuid)
    .execute(pool)
    .await
    .expect("insert live extent");

    // 4. Call cleanup with `before = far future` so the old extent qualifies.
    //    Keep the pool handle before erasing the concrete type.
    let pool_for_verify = catalog.pool().clone();
    let catalog_arc: Arc<dyn Catalog> = Arc::new(catalog);
    let app = cleanup_write_app(catalog_arc.clone());
    let resp = post_cleanup(app, "testdb", "testtable", "2030-01-01T00:00:00Z").await;

    assert_eq!(resp.status(), StatusCode::OK, "expected 200 OK from cleanup");

    let body = body_json(resp).await;
    assert_eq!(
        body["extents_deleted"].as_u64().unwrap(),
        1,
        "exactly 1 extent should be deleted"
    );
    assert_eq!(
        body["rows_freed"].as_u64().unwrap(),
        200,
        "rows_freed should equal the deleted extent's row_count"
    );
    assert_eq!(
        body["bytes_freed"].as_u64().unwrap(),
        4096,
        "bytes_freed should equal the deleted extent's byte_size"
    );

    // 5. Verify only the live extent remains.
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM extents WHERE table_id = $1")
            .bind(table_uuid)
            .fetch_one(&pool_for_verify)
            .await
            .expect("count remaining extents");
    assert_eq!(remaining, 1, "the live extent must still exist");
}

/// A soft-deleted extent with `deleted_at` after `before` is NOT cleaned up.
#[tokio::test]
async fn cleanup_respects_before_cutoff() {
    let (catalog, _container) = raw_catalog().await;

    let db_id = catalog.create_database("db2").await.expect("create_database");
    let table_id = catalog
        .create_table(db_id, "tbl2", sample_schema(), TableConfig::default())
        .await
        .expect("create_table");

    let pool = catalog.pool();
    let table_uuid = *table_id.as_uuid();
    let schema_snap_uuid: uuid::Uuid =
        sqlx::query_scalar("SELECT schema_snapshot_id FROM tables WHERE id = $1")
            .bind(table_uuid)
            .fetch_one(pool)
            .await
            .expect("fetch schema_snapshot_id");

    // Insert an extent soft-deleted 1 hour ago.
    sqlx::query(
        "INSERT INTO extents (tenant_id, table_id, schema_snapshot_id, object_path, byte_size, row_count, deleted_at)
         VALUES ($1, $2, $3, 'test/path/recent.kyma', 512, 10, now() - interval '1 hour')",
    )
    .bind(DEFAULT_TENANT.as_uuid())
    .bind(table_uuid)
    .bind(schema_snap_uuid)
    .execute(pool)
    .await
    .expect("insert recent soft-deleted extent");

    // Call cleanup with `before` = 2 days ago (the extent was deleted only 1 hour ago,
    // so it should NOT be cleaned).
    let catalog_arc: Arc<dyn Catalog> = Arc::new(catalog);
    let app = cleanup_write_app(catalog_arc.clone());
    let two_days_ago = (chrono::Utc::now() - chrono::Duration::days(2))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let resp = post_cleanup(app, "db2", "tbl2", &two_days_ago).await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(
        body["extents_deleted"].as_u64().unwrap(),
        0,
        "no extents should be cleaned when before < deleted_at"
    );
}

/// Cleanup against a nonexistent table returns 404.
#[tokio::test]
async fn cleanup_unknown_table_returns_404() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let app = cleanup_write_app(state.catalog.clone());

    let resp = post_cleanup(app, "no_such_db", "no_such_table", "2030-01-01T00:00:00Z").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Missing auth token returns 401.
#[tokio::test]
async fn cleanup_missing_auth_returns_401() {
    let state = kyma_server::test_support::seeded_state_empty().await;
    let backend: std::sync::Arc<dyn kyma_server::auth::AuthBackend> = std::sync::Arc::new(
        kyma_server::auth::EnvAuthBackend::from_str("test-write-token:write"),
    );
    let app = kyma_server::cleanup_write_router(state.catalog.clone()).layer(
        axum::middleware::from_fn_with_state(
            kyma_server::auth::AuthLayerState {
                backend,
                required: kyma_server::auth::Role::Write,
            },
            kyma_server::auth::require_role_middleware,
        ),
    );

    let req = Request::builder()
        .method("POST")
        .uri("/v1/database/db/table/tbl/cleanup?before=2030-01-01T00:00:00Z")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
