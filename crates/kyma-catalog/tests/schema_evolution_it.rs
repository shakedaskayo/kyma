//! Regression test for issue #18: `alter_table_add_column` must bind
//! `tenant_id` when inserting into `schema_snapshots`, otherwise the
//! `NOT NULL` constraint added by migration 007 fires and typed-column
//! evolution silently fails (the ingest path swallows the alter error
//! and falls back to the generic `props` column).

use arrow_schema::{DataType, Field, Schema};
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, TableConfig};
use kyma_core::tenant::TenantId;
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn fixture() -> (PostgresCatalog, testcontainers::ContainerAsync<Postgres>) {
    // pgvector/pgvector:pg16 ships the vector extension that migration 004
    // requires (mirrors tenant_isolation_it.rs).
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("connect + migrate");
    (catalog, container)
}

#[tokio::test]
async fn alter_table_add_column_succeeds_under_tenancy() {
    let (catalog, _container) = fixture().await;

    let tenant =
        TenantId::from_uuid(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());

    let db = catalog
        .create_database_in_tenant(tenant, "obs")
        .await
        .unwrap();

    let initial_schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let table_id = catalog
        .create_table_in_tenant(tenant, db, "events", initial_schema, TableConfig::default())
        .await
        .unwrap();

    // Before the fix this fails with:
    //   catalog error: sql error: null value in column "tenant_id"
    //   of relation "schema_snapshots" violates not-null constraint
    catalog
        .alter_table_add_column(table_id, "name", "string")
        .await
        .expect("schema evolution should succeed under tenancy");

    let table = catalog
        .lookup_table_in_tenant(tenant, "obs", "events")
        .await
        .unwrap();
    let names: Vec<String> = table
        .schema
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(names, vec!["id".to_string(), "name".to_string()]);
}

#[tokio::test]
async fn alter_table_add_column_succeeds_under_default_tenant() {
    // Mirrors the live kyma-bin path, which is currently tenant-blind
    // and routes through DEFAULT_TENANT via the legacy `create_table`
    // / `alter_table_add_column` defaults.
    let (catalog, _container) = fixture().await;

    let db = catalog.create_database("obs").await.unwrap();
    let initial_schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let table_id = catalog
        .create_table(db, "events", initial_schema, TableConfig::default())
        .await
        .unwrap();

    catalog
        .alter_table_add_column(table_id, "name", "string")
        .await
        .expect("schema evolution must succeed on the default-tenant path");

    let table = catalog.lookup_table("obs", "events").await.unwrap();
    assert_eq!(table.schema.fields().len(), 2);
}
