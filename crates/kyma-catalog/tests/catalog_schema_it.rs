//! Integration test for schema-listing methods in `PostgresCatalog`.
//!
//! Verifies `list_databases`, `list_tables`, and `get_table_columns` against
//! a real Postgres instance spun up via testcontainers.

use arrow_schema::{DataType, Field, Schema, TimeUnit};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, TableConfig};
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

async fn setup() -> (PostgresCatalog, testcontainers::ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_user("kyma")
        .with_password("kyma_dev")
        .with_db_name("kyma")
        .start()
        .await
        .expect("failed to start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get mapped port");
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");
    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("catalog connect + migrate");
    (catalog, container)
}

#[tokio::test]
async fn lists_databases_tables_columns() {
    let (catalog, _container) = setup().await;

    // Create database "obs".
    let db_id = catalog.create_database("obs").await.unwrap();

    // Create table "otel_logs" with three columns.
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
        .unwrap();

    // list_databases should return ["obs"].
    assert_eq!(catalog.list_databases().await.unwrap(), vec!["obs"]);

    // list_tables("obs") should return ["otel_logs"].
    assert_eq!(catalog.list_tables("obs").await.unwrap(), vec!["otel_logs"]);

    // get_table_columns should return 3 columns with correct metadata.
    let cols = catalog.get_table_columns("obs", "otel_logs").await.unwrap();
    assert_eq!(cols.len(), 3);

    assert_eq!(cols[0].name, "timestamp");
    assert_eq!(cols[0].r#type, "timestamp");
    assert!(!cols[0].nullable);

    assert_eq!(cols[1].name, "service.name");
    assert_eq!(cols[1].r#type, "string");
    assert!(!cols[1].nullable);

    assert_eq!(cols[2].name, "severity_text");
    assert_eq!(cols[2].r#type, "string");
    assert!(cols[2].nullable);
}

#[tokio::test]
async fn list_databases_empty_when_none() {
    let (catalog, _container) = setup().await;
    let dbs = catalog.list_databases().await.unwrap();
    assert!(dbs.is_empty());
}

#[tokio::test]
async fn list_tables_empty_for_empty_database() {
    let (catalog, _container) = setup().await;
    catalog.create_database("empty_db").await.unwrap();
    let tables = catalog.list_tables("empty_db").await.unwrap();
    assert!(tables.is_empty());
}

#[tokio::test]
async fn list_databases_ordered_alphabetically() {
    let (catalog, _container) = setup().await;
    catalog.create_database("zoo").await.unwrap();
    catalog.create_database("alpha").await.unwrap();
    catalog.create_database("metrics").await.unwrap();
    let dbs = catalog.list_databases().await.unwrap();
    assert_eq!(dbs, vec!["alpha", "metrics", "zoo"]);
}

#[tokio::test]
async fn get_table_columns_returns_table_not_found_for_missing_table() {
    let (catalog, _container) = setup().await;
    catalog.create_database("obs").await.unwrap();

    let err = catalog.get_table_columns("obs", "ghost").await.unwrap_err();
    match err {
        kyma_core::errors::CatalogError::TableNotFound { .. } => {}
        other => panic!("expected TableNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn list_tables_not_found_for_unknown_database() {
    let (catalog, _container) = setup().await;
    let err = catalog.list_tables("does_not_exist").await.unwrap_err();
    match err {
        kyma_core::errors::CatalogError::DatabaseNotFound(name) => {
            assert_eq!(name, "does_not_exist");
        }
        other => panic!("expected DatabaseNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn get_table_columns_not_found_for_unknown_database() {
    let (catalog, _container) = setup().await;
    let err = catalog
        .get_table_columns("does_not_exist", "some_table")
        .await
        .unwrap_err();
    match err {
        kyma_core::errors::CatalogError::DatabaseNotFound(name) => {
            assert_eq!(name, "does_not_exist");
        }
        other => panic!("expected DatabaseNotFound, got {other:?}"),
    }
}
