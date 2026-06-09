//! Artifact catalog lifecycle (migration 022): register/upsert, tenant-isolated
//! get, expiry soft-delete sweep, gc candidates, and physical row delete. These
//! back the object-store artifact layer (CI job logs, contributed files,
//! fs-watch snapshots) and its retention worker.

use chrono::{Duration, Utc};
use kyma_catalog::artifacts::ArtifactRecord;
use kyma_catalog::PostgresCatalog;
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

fn rec(tenant: TenantId, path: &str) -> ArtifactRecord {
    ArtifactRecord {
        id: None,
        tenant_id: tenant,
        object_path: path.to_string(),
        source: "github".to_string(),
        artifact_class: "log".to_string(),
        table_ref: Some("kyma.github_job_logs".to_string()),
        connector_id: None,
        size_bytes: 1234,
        sha256: Some("deadbeef".to_string()),
        created_at: None,
        expires_at: None,
        deleted_at: None,
    }
}

#[tokio::test]
async fn register_is_idempotent_and_get_is_tenant_scoped() {
    let (catalog, _c) = fixture().await;
    let t1 = TenantId::from_uuid(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
    let t2 = TenantId::from_uuid(Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());

    let path = "artifacts/t1/github/o/r/1/9.log.txt";
    let id1 = catalog.register_artifact(&rec(t1, path)).await.unwrap();

    // Re-register same (tenant, object_path) upserts in place — same row id,
    // updated size. Connectors re-tick and must not accumulate duplicate rows.
    let mut updated = rec(t1, path);
    updated.size_bytes = 9999;
    let id2 = catalog.register_artifact(&updated).await.unwrap();
    assert_eq!(id1, id2, "re-register should upsert, not insert a new row");

    let got = catalog.get_artifact_in_tenant(t1, id1).await.unwrap();
    let got = got.expect("artifact exists for its own tenant");
    assert_eq!(got.object_path, path);
    assert_eq!(got.size_bytes, 9999, "upsert should have updated size");

    // A different tenant cannot read the artifact even with the id.
    assert!(
        catalog.get_artifact_in_tenant(t2, id1).await.unwrap().is_none(),
        "artifact must be invisible across tenants"
    );
}

#[tokio::test]
async fn expiry_sweep_and_gc_candidates_and_delete() {
    let (catalog, _c) = fixture().await;
    let t = TenantId::from_uuid(Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap());
    let now = Utc::now();

    let mut past = rec(t, "artifacts/t/past.log");
    past.expires_at = Some(now - Duration::hours(1));
    let past_id = catalog.register_artifact(&past).await.unwrap();

    let mut future = rec(t, "artifacts/t/future.log");
    future.expires_at = Some(now + Duration::hours(1));
    catalog.register_artifact(&future).await.unwrap();

    // NULL expires_at = retain forever; never swept.
    catalog
        .register_artifact(&rec(t, "artifacts/t/forever.log"))
        .await
        .unwrap();

    // Pass 1: only the past-expiry row is soft-deleted and returned.
    let swept = catalog.soft_delete_expired_artifacts(now).await.unwrap();
    assert_eq!(swept.len(), 1, "only the past-expiry artifact should sweep");
    assert_eq!(swept[0].0, past_id);
    assert_eq!(swept[0].1, "artifacts/t/past.log");

    // Sweeping again is a no-op (already soft-deleted).
    assert!(catalog.soft_delete_expired_artifacts(now).await.unwrap().is_empty());

    // Pass 2: gc candidates are rows soft-deleted before the grace cutoff.
    let candidates = catalog
        .artifact_gc_candidates(now + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].0, past_id);

    // A cutoff before the soft-delete time yields nothing (grace not elapsed).
    assert!(
        catalog
            .artifact_gc_candidates(now - Duration::hours(2))
            .await
            .unwrap()
            .is_empty(),
        "grace period must protect freshly soft-deleted rows"
    );

    // Physical delete removes the row.
    catalog.delete_artifact_rows(&[past_id]).await.unwrap();
    assert!(
        catalog.get_artifact_in_tenant(t, past_id).await.unwrap().is_none(),
        "row should be gone after delete"
    );
}

#[tokio::test]
async fn list_live_artifacts_returns_only_live_for_tenant() {
    let (catalog, _c) = fixture().await;
    let t1 = TenantId::from_uuid(Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap());
    let t2 = TenantId::from_uuid(Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap());
    let now = Utc::now();

    // Register two artifacts for t1.
    catalog
        .register_artifact(&rec(t1, "artifacts/t1/live.log"))
        .await
        .unwrap();
    let mut expired = rec(t1, "artifacts/t1/expired.log");
    expired.expires_at = Some(now - Duration::hours(1));
    let expired_id = catalog.register_artifact(&expired).await.unwrap();

    // Soft-delete the expired one.
    let swept = catalog.soft_delete_expired_artifacts(now).await.unwrap();
    assert_eq!(swept.len(), 1);
    assert_eq!(swept[0].0, expired_id);

    // Register one artifact for t2 — must not appear in t1's results.
    catalog
        .register_artifact(&rec(t2, "artifacts/t2/other.log"))
        .await
        .unwrap();

    // Only the live t1 artifact is returned.
    let live = catalog.list_live_artifacts(t1).await.unwrap();
    assert_eq!(live.len(), 1, "only the non-deleted artifact should be returned");
    assert_eq!(live[0].object_path, "artifacts/t1/live.log");
    assert!(live[0].deleted_at.is_none(), "returned artifact must not be soft-deleted");

    // t2's artifact is excluded from t1's list.
    for a in &live {
        assert_ne!(a.object_path, "artifacts/t2/other.log", "cross-tenant leak");
    }

    // Symmetric check: t2 sees exactly its own artifact, and t1's count is
    // unchanged by t2's insert — catches a missing tenant predicate that the
    // one-sided check above would not.
    let live_t2 = catalog.list_live_artifacts(t2).await.unwrap();
    assert_eq!(live_t2.len(), 1, "t2 sees exactly its own live artifact");
    assert_eq!(live_t2[0].object_path, "artifacts/t2/other.log");
}
