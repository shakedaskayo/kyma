//! End-to-end file-candidate contribution: parse a Rust file → candidate
//! File/Symbol/Module nodes + DEFINES/IMPORTS/REFERENCES edges, written through
//! the real WritePath into an in-memory sqlite catalog + local format engine.

use std::sync::Arc;

use pensieve_core::catalog::Catalog;
use pensieve_core::segment_format::SegmentFormat;
use pensieve_embed::{EmbedError, EmbeddingBackend};
use pensieve_format_tlm::TelemetryFormat;
use pensieve_memory::file_candidates::{contribute, file_node_id, ContributeFile, FILE_CANDIDATES_DB};
use pensieve_memory::MemoryWriter;
use pensieve_storage::{build_object_store, StorageConfig};

#[derive(Debug)]
struct MockEmbed;

#[async_trait::async_trait]
impl EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/test"
    }
    fn dimension(&self) -> u16 {
        4
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        Ok(texts
            .iter()
            .map(|t| {
                let f = t.len() as f32;
                vec![f, f * 0.5, f * 0.25, 1.0]
            })
            .collect())
    }
}

async fn writer() -> MemoryWriter {
    let catalog: Arc<dyn Catalog> = Arc::new(
        pensieve_catalog_sqlite::SqliteCatalog::connect_in_memory()
            .await
            .expect("in-memory catalog"),
    );
    let tmp = std::env::temp_dir().join(format!("pensieve-fc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = build_object_store(&StorageConfig::Local {
        root: tmp.to_string_lossy().to_string(),
    })
    .unwrap();
    let format: Arc<dyn SegmentFormat> = Arc::new(TelemetryFormat::new(store, "test"));
    MemoryWriter::new(catalog, format, Arc::new(MockEmbed)).with_database(FILE_CANDIDATES_DB)
}

#[tokio::test]
async fn contributes_rust_file_into_candidate_subgraph() {
    let w = writer().await;
    let content = "\
use std::collections::HashMap;

fn helper() {}

fn main() {
    helper();
}
";
    let req = ContributeFile {
        path: "src/main.rs".to_string(),
        realm: "default".to_string(),
        repo: Some("acme/app".to_string()),
        content: content.to_string(),
        content_sha256: "deadbeef".to_string(),
        why_read: Some("tracing the entrypoint".to_string()),
    };

    let summary = contribute(&w, &req).await.expect("contribute");

    // Deterministic id + parsed structure.
    assert_eq!(
        summary.file_node_id,
        file_node_id("default", Some("acme/app"), "src/main.rs")
    );
    assert_eq!(summary.language, "rust");
    assert_eq!(summary.symbols, 2, "helper + main");
    assert!(summary.imports >= 1, "the `use` import");
    // 1 File + 2 Symbol + 1 Module.
    assert_eq!(summary.nodes, 4);
    // 2 DEFINES + 1 IMPORTS (+ 1 REFERENCES main→helper).
    assert!(summary.edges >= 3, "got {} edges", summary.edges);

    // Re-contributing the same file is idempotent (same id, no error).
    let again = contribute(&w, &req).await.expect("re-contribute");
    assert_eq!(again.file_node_id, summary.file_node_id);
}

#[tokio::test]
async fn non_code_file_yields_a_lone_file_node() {
    let w = writer().await;
    let req = ContributeFile {
        path: "README.md".to_string(),
        realm: "default".to_string(),
        repo: None,
        content: "# Title\n\nsome prose".to_string(),
        content_sha256: "abc".to_string(),
        why_read: None,
    };
    let summary = contribute(&w, &req).await.expect("contribute md");
    assert_eq!(summary.language, "text");
    assert_eq!(summary.symbols, 0);
    assert_eq!(summary.nodes, 1, "just the File node");
    assert_eq!(summary.edges, 0);
}
