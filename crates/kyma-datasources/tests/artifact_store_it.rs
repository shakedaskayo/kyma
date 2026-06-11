//! The data source `ArtifactStore` seam: `put_and_register` must both write the
//! blob to the object store and register its catalog tracking row, so a CI
//! job log is immediately retrievable (by bytes) and tracked (for retention).

use bytes::Bytes;
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_catalog::PostgresCatalog;
use kyma_datasources::artifacts::{ArtifactStore, ObjectArtifactStore};
use kyma_core::tenant::TenantId;
use std::sync::Arc;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

#[tokio::test]
async fn put_and_register_writes_blob_and_tracking_row() {
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.expect("connect + migrate"));

    let store = kyma_storage::build_object_store(&kyma_storage::StorageConfig::Memory).unwrap();
    let artifacts = ObjectArtifactStore::new(store.clone(), catalog.clone());

    let tenant =
        TenantId::from_uuid(Uuid::parse_str("44444444-4444-4444-4444-444444444444").unwrap());
    let path = "artifacts/t/github/o/r/7/3.log.txt";
    let record = ArtifactRecord {
        id: None,
        tenant_id: tenant,
        object_path: path.to_string(),
        source: "github".to_string(),
        artifact_class: "log".to_string(),
        table_ref: Some("kyma.github_job_logs".to_string()),
        data_source_id: None,
        size_bytes: 11,
        sha256: Some(kyma_storage::sha256_hex(b"build failed")),
        created_at: None,
        expires_at: None,
        deleted_at: None,
    };

    let id = artifacts
        .put_and_register(record, Bytes::from_static(b"build failed"))
        .await
        .expect("put_and_register");

    // Tracking row exists and is tenant-scoped.
    let got = catalog
        .get_artifact_in_tenant(tenant, id)
        .await
        .unwrap()
        .expect("artifact row registered");
    assert_eq!(got.object_path, path);
    assert_eq!(got.source, "github");

    // Blob is retrievable from the object store at the same path.
    let blob = kyma_storage::get_artifact(&store, &object_store::path::Path::from(path))
        .await
        .unwrap();
    assert_eq!(blob.as_deref(), Some(&b"build failed"[..]));
}
