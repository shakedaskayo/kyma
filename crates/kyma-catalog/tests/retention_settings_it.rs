//! Retention settings (migration 023): load/save round-trip + `restamp_artifact_expiry`
//! stamps `expires_at` per the class → source → global precedence.

use std::collections::HashMap;

use kyma_catalog::artifacts::ArtifactRecord;
use kyma_catalog::PostgresCatalog;
use kyma_core::retention::RetentionSettings;
use kyma_core::tenant::TenantId;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn fixture() -> (PostgresCatalog, testcontainers::ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg16")
        .start()
        .await
        .expect("start postgres");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = PostgresCatalog::connect(&url).await.expect("connect + migrate");
    (catalog, container)
}

fn artifact(tenant: TenantId, path: &str, class: &str) -> ArtifactRecord {
    ArtifactRecord {
        id: None,
        tenant_id: tenant,
        object_path: path.to_string(),
        source: "github".into(),
        artifact_class: class.into(),
        table_ref: None,
        connector_id: None,
        size_bytes: 1,
        sha256: None,
        created_at: None,
        expires_at: None,
        deleted_at: None,
    }
}

#[tokio::test]
async fn settings_round_trip_and_restamp_applies_precedence() {
    let (catalog, _c) = fixture().await;
    let tenant =
        TenantId::from_uuid(Uuid::parse_str("77777777-7777-7777-7777-777777777777").unwrap());

    // Defaults when unset = retain forever.
    let loaded = catalog.load_retention_settings(tenant).await.unwrap();
    assert_eq!(loaded, RetentionSettings::default());

    // Two artifacts: a log (class rule) and a file (falls to source rule).
    let log_id = catalog
        .register_artifact(&artifact(tenant, "artifacts/t/a.log", "log"))
        .await
        .unwrap();
    let file_id = catalog
        .register_artifact(&artifact(tenant, "artifacts/t/b.txt", "file"))
        .await
        .unwrap();

    // Both start with no expiry.
    assert!(catalog.get_artifact_in_tenant(tenant, log_id).await.unwrap().unwrap().expires_at.is_none());

    let settings = RetentionSettings {
        global_default_days: Some(90),
        per_source_days: HashMap::from([("github".to_string(), 30)]),
        per_artifact_class_days: HashMap::from([("log".to_string(), 14)]),
        ..Default::default()
    };
    catalog.save_retention_settings(tenant, &settings).await.unwrap();
    assert_eq!(catalog.load_retention_settings(tenant).await.unwrap(), settings);

    let touched = catalog.restamp_artifact_expiry(tenant, &settings).await.unwrap();
    assert_eq!(touched, 2);

    // log → class rule (14d); file → source rule (30d).
    let log = catalog.get_artifact_in_tenant(tenant, log_id).await.unwrap().unwrap();
    let file = catalog.get_artifact_in_tenant(tenant, file_id).await.unwrap().unwrap();
    assert_eq!((log.expires_at.unwrap() - log.created_at.unwrap()).num_days(), 14);
    assert_eq!((file.expires_at.unwrap() - file.created_at.unwrap()).num_days(), 30);

    // Idempotent: a second restamp touches nothing.
    assert_eq!(catalog.restamp_artifact_expiry(tenant, &settings).await.unwrap(), 0);

    // Clearing the rules reverts to retain-forever (expires_at NULL).
    catalog
        .restamp_artifact_expiry(tenant, &RetentionSettings::default())
        .await
        .unwrap();
    let log = catalog.get_artifact_in_tenant(tenant, log_id).await.unwrap().unwrap();
    assert!(log.expires_at.is_none(), "cleared retention should retain forever");
}
