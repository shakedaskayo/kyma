//! Slice 0 verification gate: cross-tenant isolation.

use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::errors::{CatalogError, Error};
use kyma_core::tenant::TenantId;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn fixture() -> (PostgresCatalog, testcontainers::ContainerAsync<Postgres>) {
    // pgvector/pgvector:pg16 ships the vector extension that migration 004
    // requires. Plain `postgres:*` images don't have it, so they fail
    // migration before our tenant-scoped tables are even created.
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = PostgresCatalog::connect(&url).await.expect("connect + migrate");
    (catalog, container)
}

#[tokio::test]
async fn same_database_name_under_two_tenants_does_not_cross() {
    let (catalog, _container) = fixture().await;

    let tenant_a = TenantId::from_uuid(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
    let tenant_b = TenantId::from_uuid(Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());

    let db_a = catalog.create_database_in_tenant(tenant_a, "default").await.unwrap();
    let db_b = catalog.create_database_in_tenant(tenant_b, "default").await.unwrap();
    assert_ne!(db_a.as_uuid(), db_b.as_uuid());

    let a_dbs = catalog.list_databases_in_tenant(tenant_a).await.unwrap();
    let b_dbs = catalog.list_databases_in_tenant(tenant_b).await.unwrap();
    assert_eq!(a_dbs, vec!["default".to_string()]);
    assert_eq!(b_dbs, vec!["default".to_string()]);

    let tenant_c = TenantId::from_uuid(Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap());
    let c_dbs = catalog.list_databases_in_tenant(tenant_c).await.unwrap();
    assert!(c_dbs.is_empty());
}

#[tokio::test]
async fn lookup_table_never_crosses_tenants() {
    use arrow_schema::{DataType, Field, Schema};
    use kyma_core::catalog::TableConfig;
    use std::sync::Arc;

    let (catalog, _container) = fixture().await;

    let tenant_a = TenantId::from_uuid(Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap());
    let tenant_b = TenantId::from_uuid(Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap());

    let schema_a = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let schema_b = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("payload", DataType::Utf8, true),
    ]));

    let db_a = catalog.create_database_in_tenant(tenant_a, "default").await.unwrap();
    catalog.create_table_in_tenant(tenant_a, db_a, "events", schema_a, TableConfig::default())
        .await.unwrap();

    let db_b = catalog.create_database_in_tenant(tenant_b, "default").await.unwrap();
    catalog.create_table_in_tenant(tenant_b, db_b, "events", schema_b, TableConfig::default())
        .await.unwrap();

    let a = catalog.lookup_table_in_tenant(tenant_a, "default", "events").await.unwrap();
    assert_eq!(a.schema.fields().len(), 1);

    let b = catalog.lookup_table_in_tenant(tenant_b, "default", "events").await.unwrap();
    assert_eq!(b.schema.fields().len(), 2);

    let tenant_c = TenantId::from_uuid(Uuid::parse_str("cccccccc-cccc-cccc-cccc-cccccccccccc").unwrap());
    let err = catalog.lookup_table_in_tenant(tenant_c, "default", "events")
        .await.expect_err("tenant c must not see other tenants' tables");
    match err {
        Error::Catalog(CatalogError::DatabaseNotFound(_)) | Error::Catalog(CatalogError::TableNotFound { .. }) => {}
        other => panic!("expected DatabaseNotFound or TableNotFound, got {other:?}"),
    }
}
