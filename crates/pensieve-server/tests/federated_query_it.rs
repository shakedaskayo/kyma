//! Integration tests for federated (live-proxied) tables in the query path.
//!
//! A federated table is a metadata-only catalog entry (`TableConfig.federated`
//! = Some) whose rows live on an external platform — see `kyma-federation`.
//! Without a live Microsoft Fabric endpoint these tests can't run a remote
//! query end-to-end; what they DO pin down is the wiring around it:
//!
//!   - the query handler detects federated tables and (when the server has no
//!     federation runtime) fails loudly instead of returning empty results;
//!   - local tables in the same database keep working;
//!   - ingest into a federated table is rejected;
//!   - `exclude_from_wildcard` drops the table from `x-database: *`
//!     resolution while explicit access keeps seeing it;
//!   - `drop_table` removes a federated table (the metadata-sync drift path).

#![cfg(feature = "test-support")]

use arrow_schema::{DataType, Field, Schema};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_core::catalog::{FederatedTableSpec, TableConfig};
use std::sync::Arc;
use tower::ServiceExt;

fn federated_config(exclude_from_wildcard: bool) -> TableConfig {
    TableConfig {
        federated: Some(FederatedTableSpec {
            platform: "msfabric".into(),
            endpoint: "example.datawarehouse.fabric.microsoft.com".into(),
            remote_database: "lakehouse1".into(),
            remote_schema: "dbo".into(),
            remote_table: "orders".into(),
            credential_id: uuid::Uuid::new_v4(),
            exclude_from_wildcard,
        }),
        ..TableConfig::default()
    }
}

fn orders_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount", DataType::Float64, true),
    ]))
}

/// Seed `obs.fab_orders` as a federated table next to the fixture's local
/// `otel_logs`.
async fn seeded_state_with_federated(
    exclude_from_wildcard: bool,
) -> kyma_server::QueryState {
    let state = kyma_server::test_support::seeded_state_with_obs_otel_logs().await;
    let obs_id = state
        .catalog
        .lookup_database("obs")
        .await
        .expect("lookup_database obs")
        .expect("obs database exists");
    state
        .catalog
        .create_table(
            obs_id,
            "fab_orders",
            orders_schema(),
            federated_config(exclude_from_wildcard),
        )
        .await
        .expect("create_table fab_orders");
    state
}

async fn run(state: kyma_server::QueryState, req: Request<Body>) -> (StatusCode, String) {
    let app = kyma_server::router(state);
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn query_req(database: &str, sql: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/query")
        .header("content-type", "application/sql")
        .header("x-database", database)
        .body(Body::from(sql.to_owned()))
        .unwrap()
}

#[tokio::test]
async fn federated_table_without_runtime_fails_loudly() {
    let state = seeded_state_with_federated(false).await;
    // state.federation is None — querying the database that contains a
    // federated table must NOT silently treat it as empty.
    let (status, body) = run(state, query_req("obs", "SELECT id FROM fab_orders")).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body}");
    assert!(
        body.contains("federation"),
        "error should name the federation gap: {body}"
    );
}

#[tokio::test]
async fn ingest_into_federated_table_is_rejected() {
    let state = seeded_state_with_federated(false).await;
    let table = state
        .catalog
        .lookup_table("obs", "fab_orders")
        .await
        .expect("lookup fab_orders");

    let write = kyma_ingest_core::WritePath::new(state.catalog.clone(), state.format.clone());
    let batch = arrow_array::RecordBatch::try_new(
        orders_schema(),
        vec![
            Arc::new(arrow_array::Int64Array::from(vec![1_i64])),
            Arc::new(arrow_array::Float64Array::from(vec![Some(9.5_f64)])),
        ],
    )
    .unwrap();

    let err = write
        .ingest("obs", &table, vec![batch])
        .await
        .expect_err("ingest into a federated table must fail");
    assert!(
        err.to_string().contains("federated"),
        "error should say the table is federated: {err}"
    );
}

#[tokio::test]
async fn wildcard_resolution_honors_exclude_flag() {
    use kyma_server::query_multidb::resolve_all_db_tables;

    // Excluded: the wildcard sweep must not see fab_orders…
    let state = seeded_state_with_federated(true).await;
    let tables = resolve_all_db_tables(&state.catalog, kyma_core::tenant::DEFAULT_TENANT, None)
        .await
        .expect("resolve");
    assert!(
        !tables.iter().any(|dt| dt.table.name == "fab_orders"),
        "excluded federated table leaked into the wildcard set"
    );
    // …while the local fixture table is still there.
    assert!(tables.iter().any(|dt| dt.table.name == "otel_logs"));
    // Explicit single-database listing still sees it.
    let names = state.catalog.list_tables("obs").await.expect("list obs");
    assert!(names.contains(&"fab_orders".to_string()));

    // Not excluded: the wildcard sweep includes it.
    let state = seeded_state_with_federated(false).await;
    let tables = resolve_all_db_tables(&state.catalog, kyma_core::tenant::DEFAULT_TENANT, None)
        .await
        .expect("resolve");
    assert!(tables.iter().any(|dt| dt.table.name == "fab_orders"));
}

#[tokio::test]
async fn drop_table_removes_federated_table() {
    let state = seeded_state_with_federated(false).await;
    let dropped = state
        .catalog
        .drop_table("obs", "fab_orders")
        .await
        .expect("drop_table");
    assert!(dropped);
    let names = state.catalog.list_tables("obs").await.expect("list obs");
    assert!(!names.contains(&"fab_orders".to_string()));
    // Idempotent second drop returns false.
    let dropped_again = state
        .catalog
        .drop_table("obs", "fab_orders")
        .await
        .expect("drop_table again");
    assert!(!dropped_again);
}
