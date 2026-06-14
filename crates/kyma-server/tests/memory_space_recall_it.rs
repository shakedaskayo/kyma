//! Execution test for S3.3 space-scoped recall: writes a public and an
//! agent-private memory, then runs the real `recall_sql` through `POST
//! /v1/query` over the embedded engine (in-memory SQLite catalog + local
//! segment format + a mock embedder — the same DataFusion engine + SQL server
//! and `kyma local` execute, no Docker). Confirms the `space` projection and
//! predicate actually execute and that visibility is enforced:
//! - agent B sees the public memory but NOT agent A's private one,
//! - agent A sees its own private memory,
//! - no `space_agent` (default) sees everything (unchanged behavior).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_embed::EmbeddingBackend as _;
use kyma_memory::types::RecallFilter;
use kyma_memory::{sql, CreateMemory, MemoryWriter, NODE_TABLE};
use tower::ServiceExt;

#[derive(Debug)]
struct MockEmbed;

#[async_trait::async_trait]
impl kyma_embed::EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/space-recall"
    }
    fn dimension(&self) -> u16 {
        4
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, kyma_embed::EmbedError> {
        // Deterministic + fractional (whole-number floats would render as
        // integer literals and the cosine UDF rejects Int64 arrays). Identical
        // content embeds identically, so only `space` distinguishes recall.
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

fn query_app(catalog: CatalogArc, format: FormatArc) -> axum::Router {
    // No auth layer → no Principal → default tenant, no scope gate (fine for a
    // single-database embedded test).
    let state = kyma_server::QueryState {
        catalog,
        format,
        schema_cache: Arc::new(kyma_server::catalog_handler::SchemaCache::default()),
        node_id: None,
        pg_pool: None,
        federation: None,
        layout_cache: Arc::new(kyma_server::graph_layout_cache::LayoutCache::new()),
    };
    kyma_server::router(state)
}

fn private_to(agent: &str) -> CreateMemory {
    let mut m = CreateMemory::new("shared note");
    m.space = Some(format!("private:{agent}"));
    m
}

/// Run `sql` via POST /v1/query and return the `id` of every returned row.
async fn query_ids(app: &axum::Router, sql: &str) -> Vec<String> {
    let req = Request::builder()
        .method("POST")
        .uri("/v1/query")
        .header("content-type", "text/plain")
        .header("x-database", kyma_memory::DEFAULT_DATABASE)
        .body(Body::from(sql.to_string()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert_eq!(status, StatusCode::OK, "query should succeed; body: {text}");
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v.get("id").and_then(|i| i.as_str().map(str::to_string)))
        .collect()
}

#[tokio::test]
async fn recall_enforces_private_space_visibility() {
    let tmp = tempfile::tempdir().unwrap();
    let (catalog, format) = embedded_engine(tmp.path()).await;

    let writer = MemoryWriter::new(catalog.clone(), format.clone(), Arc::new(MockEmbed));
    // One public memory, one private to agent A. Identical content → identical
    // embedding, so only the space predicate distinguishes them in recall.
    let pub_id = format!(
        "memory:{}",
        writer
            .save(&CreateMemory::new("shared note"))
            .await
            .unwrap()
    );
    let priv_id = format!(
        "memory:{}",
        writer.save(&private_to("agentA")).await.unwrap()
    );

    let app = query_app(catalog, format);
    let qvec = MockEmbed.embed(&["shared note".to_string()]).await.unwrap()[0].clone();

    // Agent B: sees the public memory, NOT agent A's private one.
    let f_b = RecallFilter {
        space_agent: Some("agentB".to_string()),
        ..Default::default()
    };
    let ids_b = query_ids(&app, &sql::recall_sql(NODE_TABLE, &qvec, &f_b, 10, None)).await;
    assert!(
        ids_b.contains(&pub_id),
        "agent B sees the public memory: {ids_b:?}"
    );
    assert!(
        !ids_b.contains(&priv_id),
        "agent B must NOT see agent A's private memory: {ids_b:?}"
    );

    // Agent A: sees its own private memory (and the public one).
    let f_a = RecallFilter {
        space_agent: Some("agentA".to_string()),
        ..Default::default()
    };
    let ids_a = query_ids(&app, &sql::recall_sql(NODE_TABLE, &qvec, &f_a, 10, None)).await;
    assert!(
        ids_a.contains(&priv_id),
        "agent A sees its own private memory: {ids_a:?}"
    );
    assert!(
        ids_a.contains(&pub_id),
        "agent A also sees the public memory: {ids_a:?}"
    );

    // No agent context (default): no space filter → both visible (unchanged).
    let f_none = RecallFilter::default();
    let ids_none = query_ids(&app, &sql::recall_sql(NODE_TABLE, &qvec, &f_none, 10, None)).await;
    assert!(
        ids_none.contains(&pub_id) && ids_none.contains(&priv_id),
        "default sees both: {ids_none:?}"
    );

    // The candidate-id (ANN) recall path enforces the same visibility.
    let ids_b_ann = query_ids(
        &app,
        &sql::recall_sql_for_ids(
            NODE_TABLE,
            &qvec,
            &f_b,
            10,
            &[pub_id.clone(), priv_id.clone()],
        ),
    )
    .await;
    assert!(ids_b_ann.contains(&pub_id));
    assert!(
        !ids_b_ann.contains(&priv_id),
        "ANN path also hides A's private memory from B"
    );
}
