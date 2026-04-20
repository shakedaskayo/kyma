//! Test fixtures for `kyma-server` integration tests.
//!
//! Builds a real [`QueryState`] against a testcontainers Postgres instance
//! with an in-memory object store, and seeds it with an "obs" database +
//! "otel_logs" table that the catalog HTTP tests exercise.

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, TableConfig};
use kyma_format_tlm::TelemetryFormat;
use object_store::memory::InMemory;
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

use crate::QueryState;
use crate::catalog_handler::SchemaCache;

/// Spin up a fresh Postgres via testcontainers, seed it with the "obs" /
/// "otel_logs" table, and return a [`QueryState`] ready for use in handler
/// tests.
///
/// The container's Drop is suppressed (`mem::forget`) so it outlives the
/// test's tokio runtime; a fresh container is spun up for each call.
///
/// To test auth behaviour, callers should wrap the router from
/// [`crate::router`] with the auth middleware using
/// [`crate::auth::AuthConfig::from_str`] directly (no env var needed).
///
/// # Panics
///
/// Panics on any fixture-setup failure.
pub async fn seeded_state_with_obs_otel_logs() -> QueryState {
    // Spin up Postgres via testcontainers.
    let container = Postgres::default()
        .with_user("kyma")
        .with_password("kyma_dev")
        .with_db_name("kyma")
        .start()
        .await
        .expect("testcontainers: failed to start Postgres");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("testcontainers: failed to get mapped port");
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");

    let catalog: Arc<dyn Catalog> = Arc::new(
        PostgresCatalog::connect(&url)
            .await
            .expect("catalog connect + migrate"),
    );

    // Seed: database "obs" + table "otel_logs" with 3 columns.
    let db_id = catalog
        .create_database("obs")
        .await
        .expect("create_database obs");
    let schema = Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            false,
        ),
        Field::new("service.name", DataType::Utf8, false),
        Field::new("severity_text", DataType::Utf8, true),
    ]));
    catalog
        .create_table(db_id, "otel_logs", schema, TableConfig::default())
        .await
        .expect("create_table otel_logs");

    // In-memory object store — no S3/MinIO needed for schema-only tests.
    let store = Arc::new(InMemory::new());
    let format = Arc::new(TelemetryFormat::new(store, "kyma-test"));

    // Suppress Drop so the container outlives the test's tokio runtime.
    std::mem::forget(container);

    QueryState {
        catalog,
        format,
        schema_cache: Arc::new(SchemaCache::new()),
    }
}
