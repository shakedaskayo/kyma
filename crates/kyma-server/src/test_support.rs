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
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

use crate::catalog_handler::SchemaCache;
use crate::QueryState;

/// A running test HTTP server bound to an ephemeral port on `127.0.0.1`.
///
/// Constructed via [`start_test_server_with_seeded_data`].
pub struct TestServer {
    base_url: String,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// Returns the HTTP base URL, e.g. `http://127.0.0.1:54321`.
    pub fn http_base_url(&self) -> &str {
        &self.base_url
    }

    /// Signal the server to stop and wait for it to exit.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.handle.await;
    }
}

/// Spin up a fresh Postgres via testcontainers with no databases/tables,
/// returning a bare [`QueryState`]. Useful for dashboard and other tests
/// that don't need any table data.
///
/// # Panics
///
/// Panics on any fixture-setup failure.
pub async fn seeded_state_empty() -> QueryState {
    let container = Postgres::default()
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
    std::env::set_var("KYMA_TEST_DATABASE_URL", &url);

    let catalog: Arc<dyn Catalog> = Arc::new(
        PostgresCatalog::connect(&url)
            .await
            .expect("catalog connect + migrate"),
    );

    let store = Arc::new(InMemory::new());
    let format = Arc::new(TelemetryFormat::new(store, "kyma-test"));

    // Suppress Drop so the container outlives the test's tokio runtime.
    std::mem::forget(container);

    QueryState {
        catalog,
        format,
        schema_cache: Arc::new(SchemaCache::default()),
        node_id: None,
    }
}

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
    std::env::set_var("KYMA_TEST_DATABASE_URL", &url);

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
        schema_cache: Arc::new(SchemaCache::default()),
        node_id: None,
    }
}

/// Spin up a fresh Postgres via testcontainers and seed it with TWO
/// databases:
///
/// - `obs.otel_logs` (3 columns — see [`seeded_state_with_obs_otel_logs`])
/// - `stg.http_reqs` (`timestamp`, `status`, `path`)
///
/// Both tables are **schema-only** — no rows are ingested. This is enough
/// for cross-source plan/scope/error assertions in the Discover endpoint
/// integration tests; the executor still runs end-to-end and emits a
/// `source_done` per source.
///
/// # Panics
///
/// Panics on any fixture-setup failure.
pub async fn seeded_state_two_databases() -> QueryState {
    let state = seeded_state_with_obs_otel_logs().await;
    let db_id = state
        .catalog
        .create_database("stg")
        .await
        .expect("create_database stg");
    let schema = Arc::new(Schema::new(vec![
        Field::new(
            "timestamp",
            DataType::Timestamp(TimeUnit::Nanosecond, None),
            false,
        ),
        Field::new("status", DataType::Int64, false),
        Field::new("path", DataType::Utf8, false),
    ]));
    state
        .catalog
        .create_table(db_id, "http_reqs", schema, TableConfig::default())
        .await
        .expect("create_table stg.http_reqs");
    state
}

/// Spin up a full HTTP server (query + flight-web routes) against a
/// testcontainers Postgres with seeded "obs"/"otel_logs" data.
///
/// Auth is configured with a single read token: `test-read-token:read`.
///
/// The returned [`TestServer`] gives you `http_base_url()` (bound to an
/// OS-assigned ephemeral port) and `shutdown().await` to cleanly stop it.
///
/// # Panics
///
/// Panics on any fixture-setup failure.
#[cfg(feature = "web-ui")]
pub async fn start_test_server_with_seeded_data() -> TestServer {
    let state = seeded_state_with_obs_otel_logs().await;

    let backend: std::sync::Arc<dyn crate::auth::AuthBackend> = std::sync::Arc::new(
        crate::auth::EnvAuthBackend::from_str("test-read-token:read"),
    );

    // Build the query router with auth.
    let query_router = crate::router(state.clone()).layer(axum::middleware::from_fn_with_state(
        crate::auth::AuthLayerState {
            backend: backend.clone(),
            required: crate::auth::Role::Read,
        },
        crate::auth::require_role_middleware,
    ));

    // Build the flight-web router with auth.
    let flight_router =
        crate::flight_web_router(state).layer(axum::middleware::from_fn_with_state(
            crate::auth::AuthLayerState {
                backend: backend.clone(),
                required: crate::auth::Role::Read,
            },
            crate::auth::require_role_middleware,
        ));

    let app = query_router.merge(flight_router);

    // Bind an ephemeral port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let base_url = format!("http://{addr}");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });

    TestServer {
        base_url,
        shutdown_tx,
        handle,
    }
}
