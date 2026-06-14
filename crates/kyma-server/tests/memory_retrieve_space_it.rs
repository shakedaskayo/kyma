//! Execution test for the recall-layer space wiring (S3.3): drives the real
//! `memory_retrieve::retrieve()` over the embedded engine (in-memory SQLite +
//! local segment format), with a deterministic mock embedder injected via
//! `kyma_memory::try_set_shared_embedding` so the path runs offline (no ONNX).
//! Confirms `RetrieveRequest.space_agent` flows through to the recall filter and
//! is enforced end-to-end: agent B's recall excludes agent A's private memory.
//!
//! Own test binary ⇒ the process-global embedding OnceCell is seeded only here.

use std::sync::Arc;

use kyma_memory::{CreateMemory, MemoryWriter};
use kyma_server::agent::memory_retrieve::{retrieve, RetrieveRequest};
use kyma_server::agent::tools::SharedToolCtx;

#[derive(Debug)]
struct MockEmbed;

#[async_trait::async_trait]
impl kyma_embed::EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/retrieve-space"
    }
    fn dimension(&self) -> u16 {
        4
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, kyma_embed::EmbedError> {
        // Fractional so make_array renders floats (the cosine UDF rejects Int64).
        Ok(texts
            .iter()
            .map(|t| vec![0.5, 0.25, 0.125, t.len() as f32 / 100.0])
            .collect())
    }
}

type CatalogArc = Arc<dyn kyma_core::catalog::Catalog>;
type FormatArc = Arc<dyn kyma_core::segment_format::SegmentFormat>;

async fn embedded_engine(root: &std::path::Path) -> (CatalogArc, FormatArc) {
    let catalog: CatalogArc = Arc::new(
        kyma_catalog_sqlite::SqliteCatalog::connect_in_memory()
            .await
            .expect("in-memory catalog"),
    );
    let store = kyma_storage::build_object_store(&kyma_storage::StorageConfig::Local {
        root: root.to_string_lossy().to_string(),
    })
    .expect("local store");
    let format: FormatArc = Arc::new(kyma_format_tlm::TelemetryFormat::new(store, "test"));
    (catalog, format)
}

fn ctx(catalog: CatalogArc, format: FormatArc) -> SharedToolCtx {
    SharedToolCtx {
        catalog,
        format,
        pool: None,
        memory: None,
        hitl: None,
        federation: None,
    }
}

fn req_for(agent: Option<&str>) -> RetrieveRequest {
    RetrieveRequest {
        query: "shared note".into(),
        realms: Vec::new(),
        memory_type: None,
        tags: Vec::new(),
        importance_min: None,
        as_of: None,
        include_invalidated: false,
        limit: Some(10),
        expand_hops: Some(0),
        space_agent: agent.map(str::to_string),
    }
}

#[tokio::test]
async fn retrieve_enforces_space_agent_visibility() {
    // Seed the deterministic embedder before any recall path builds the global.
    assert!(
        kyma_memory::try_set_shared_embedding(Arc::new(MockEmbed)),
        "embedder must be settable (first use in this test binary)"
    );

    let tmp = tempfile::tempdir().unwrap();
    let (catalog, format) = embedded_engine(tmp.path()).await;

    let writer = MemoryWriter::new(catalog.clone(), format.clone(), Arc::new(MockEmbed));
    let pub_id = format!(
        "memory:{}",
        writer
            .save(&CreateMemory::new("shared note"))
            .await
            .unwrap()
    );
    let priv_id = {
        let mut m = CreateMemory::new("shared note");
        m.space = Some("private:agentA".into());
        format!("memory:{}", writer.save(&m).await.unwrap())
    };

    let shared = ctx(catalog, format);
    let ids = |r: &kyma_server::agent::memory_retrieve::RetrieveResult| {
        r.memories.iter().map(|m| m.id.clone()).collect::<Vec<_>>()
    };

    // Agent B: sees public, NOT agent A's private.
    let r_b = retrieve(&shared, &req_for(Some("agentB"))).await;
    let b = ids(&r_b);
    assert!(b.contains(&pub_id), "agent B sees the public memory: {b:?}");
    assert!(
        !b.contains(&priv_id),
        "agent B must NOT see agent A's private memory: {b:?}"
    );

    // Agent A: sees its own private memory.
    let r_a = retrieve(&shared, &req_for(Some("agentA"))).await;
    assert!(
        ids(&r_a).contains(&priv_id),
        "agent A sees its own private memory: {:?}",
        ids(&r_a)
    );

    // No agent context: no filter → both visible (unchanged behavior).
    let r_none = retrieve(&shared, &req_for(None)).await;
    let n = ids(&r_none);
    assert!(
        n.contains(&pub_id) && n.contains(&priv_id),
        "default recall sees both: {n:?}"
    );
}
