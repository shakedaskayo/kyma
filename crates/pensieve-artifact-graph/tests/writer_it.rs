use std::sync::Arc;

use chrono::Utc;
use pensieve_artifact_graph::{artifact_node_row, ArtifactGraphWriter, ARTIFACTS_DB, GRAPH_NAME};
use pensieve_catalog::artifacts::ArtifactRecord;
use pensieve_core::catalog::{Catalog, GraphSpec};
use pensieve_core::segment_format::SegmentFormat;
use pensieve_core::tenant::TenantId;
use pensieve_format_tlm::TelemetryFormat;
use pensieve_storage::{build_object_store, StorageConfig};
use uuid::Uuid;

async fn writer() -> (Arc<dyn Catalog>, ArtifactGraphWriter) {
    let catalog: Arc<dyn Catalog> = Arc::new(
        pensieve_catalog_sqlite::SqliteCatalog::connect_in_memory().await.unwrap(),
    );
    let tmp = std::env::temp_dir().join(format!("pensieve-ag-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = build_object_store(&StorageConfig::Local {
        root: tmp.to_string_lossy().to_string(),
    })
    .unwrap();
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store, "test"));
    (catalog.clone(), ArtifactGraphWriter::new(catalog, format))
}

fn rec(source: &str, class: &str, path: &str) -> ArtifactRecord {
    ArtifactRecord {
        id: Some(Uuid::new_v4()),
        tenant_id: TenantId::from_uuid(Uuid::nil()),
        object_path: path.into(),
        source: source.into(),
        artifact_class: class.into(),
        table_ref: None,
        data_source_id: None,
        size_bytes: 42,
        sha256: Some("abc".into()),
        created_at: Some(Utc::now()),
        expires_at: None,
        deleted_at: None,
    }
}

#[test]
fn node_row_shape_is_artifact_labeled() {
    let r = rec("fswatch", "file", "snapshots/x.txt");
    let node = artifact_node_row(&r);
    assert_eq!(node["labels"], "Artifact");
    assert_eq!(node["id"], format!("artifact::{}", r.id.unwrap()));
    let props: serde_json::Value =
        serde_json::from_str(node["props"].as_str().unwrap()).unwrap();
    assert_eq!(props["artifact_class"], "file");
    assert_eq!(props["source"], "fswatch");
    assert_eq!(props["object_path"], "snapshots/x.txt");
    assert_eq!(props["retrievable"], true);
}

#[tokio::test]
async fn provision_then_sync_registers_graph_and_writes_nodes() {
    let (catalog, w) = writer().await;
    let records = vec![
        rec("fswatch", "file", "snapshots/x.txt"),
        rec("agent", "file", "uploads/y.bin"),
        rec("github", "log", "artifacts/t/github/a/b/1/2.log.txt"),
    ];
    let written = w.sync(&records).await.unwrap();
    assert_eq!(written, 2, "github record is skipped");

    let g = catalog.get_graph(ARTIFACTS_DB, GRAPH_NAME).await.unwrap();
    assert!(g.is_some(), "artifacts graph registered");

    let again = w.sync(&records).await.unwrap();
    assert_eq!(again, 2);

    w.ensure_provisioned().await.unwrap();
    let _ = GraphSpec::with_defaults("artifact_nodes", "artifact_edges");
}

#[test]
fn deleted_record_has_retrievable_false() {
    let mut r = rec("fswatch", "file", "snapshots/deleted.txt");
    r.deleted_at = Some(Utc::now());
    let node = artifact_node_row(&r);
    let props: serde_json::Value =
        serde_json::from_str(node["props"].as_str().unwrap()).unwrap();
    assert_eq!(props["retrievable"], false, "deleted record must be retrievable=false");
}

#[tokio::test]
async fn sync_skips_deleted_records() {
    let (_catalog, w) = writer().await;
    let mut r = rec("agent", "file", "uploads/gone.bin");
    r.deleted_at = Some(Utc::now());
    let written = w.sync(&[r]).await.unwrap();
    assert_eq!(written, 0, "deleted record must be filtered out by sync");
}

#[test]
fn none_id_falls_back_to_object_path() {
    let mut r = rec("fswatch", "file", "snapshots/no-id.txt");
    r.id = None;
    let node = artifact_node_row(&r);
    assert_eq!(
        node["id"],
        format!("artifact::{}", "snapshots/no-id.txt"),
        "None id must fall back to artifact::object_path"
    );
}

#[test]
fn node_row_includes_artifact_id_and_created_at() {
    let r = rec("agent", "file", "uploads/check.bin");
    let expected_id = r.id.unwrap();
    let node = artifact_node_row(&r);
    let props: serde_json::Value =
        serde_json::from_str(node["props"].as_str().unwrap()).unwrap();
    assert_eq!(
        props["artifact_id"],
        expected_id.to_string(),
        "artifact_id prop must be the UUID string"
    );
    assert!(
        props["created_at"].is_string(),
        "created_at prop must be present as an RFC3339 string"
    );
}
