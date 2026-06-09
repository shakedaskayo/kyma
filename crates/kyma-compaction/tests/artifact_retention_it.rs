//! ArtifactRetentionWorker end-to-end: an expired artifact's blob is deleted
//! from the object store and its catalog row removed; an unexpired artifact
//! survives.

use std::sync::Arc;

use bytes::Bytes;
use chrono::{Duration, Utc};
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_catalog::PostgresCatalog;
use kyma_compaction::ArtifactRetentionWorker;
use kyma_core::catalog::Catalog;
use kyma_core::tenant::TenantId;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::ObjectStore;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

fn rec(tenant: TenantId, path: &str, expires_at: chrono::DateTime<Utc>) -> ArtifactRecord {
    ArtifactRecord {
        id: None,
        tenant_id: tenant,
        object_path: path.to_string(),
        source: "github".into(),
        artifact_class: "log".into(),
        table_ref: None,
        connector_id: None,
        size_bytes: 3,
        sha256: None,
        created_at: None,
        expires_at: Some(expires_at),
        deleted_at: None,
    }
}

#[tokio::test]
async fn expired_artifact_blob_and_row_are_swept() {
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pg = Arc::new(PostgresCatalog::connect(&url).await.expect("connect + migrate"));
    let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());

    let tenant =
        TenantId::from_uuid(Uuid::parse_str("66666666-6666-6666-6666-666666666666").unwrap());
    let now = Utc::now();

    // Expired artifact + its blob.
    let expired_path = "artifacts/t/expired.log";
    let expired_id = pg
        .register_artifact(&rec(tenant, expired_path, now - Duration::hours(2)))
        .await
        .unwrap();
    store
        .put(&Path::from(expired_path), Bytes::from_static(b"old").into())
        .await
        .unwrap();

    // Live artifact (expires in the future) + its blob — must survive.
    let live_path = "artifacts/t/live.log";
    let live_id = pg
        .register_artifact(&rec(tenant, live_path, now + Duration::hours(2)))
        .await
        .unwrap();
    store
        .put(&Path::from(live_path), Bytes::from_static(b"new").into())
        .await
        .unwrap();

    let mut worker = ArtifactRetentionWorker::new(pg.clone() as Arc<dyn Catalog>, store.clone());
    // Negative grace so the just-soft-deleted row is immediately gc-eligible in
    // the same tick (cutoff = now - (-1s) = now + 1s > deleted_at).
    worker.grace_period = Duration::seconds(-1);

    let stats = worker.tick().await.unwrap();
    assert_eq!(stats.soft_deleted, 1, "only the expired artifact soft-deletes");
    assert_eq!(stats.blobs_deleted, 1);
    assert_eq!(stats.rows_deleted, 1);

    // Expired: blob gone, row gone.
    assert!(
        matches!(
            store.head(&Path::from(expired_path)).await,
            Err(object_store::Error::NotFound { .. })
        ),
        "expired blob should be deleted"
    );
    assert!(
        pg.get_artifact_in_tenant(tenant, expired_id).await.unwrap().is_none(),
        "expired row should be deleted"
    );

    // Live: blob + row intact.
    assert!(store.head(&Path::from(live_path)).await.is_ok(), "live blob survives");
    assert!(
        pg.get_artifact_in_tenant(tenant, live_id).await.unwrap().is_some(),
        "live row survives"
    );
}
