//! End-to-end storage + HTTP check for `GET /v1/agent/memory/source-summary`:
//! memories grouped by `provenance.source` + realm, latest-version dedup,
//! archived versions excluded, and missing provenance reported as "manual".
//!
//! Runs over the embedded engine (in-memory SQLite catalog + local segment
//! format + a mock embedder) — the same DataFusion engine and SQL that both
//! server mode (Postgres catalog) and `pensieve local` execute, so this covers the
//! aggregate's behavior for both modes without Docker or an ONNX model.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use pensieve_core::tenant::DEFAULT_TENANT;
use pensieve_memory::{CreateMemory, MemoryWriter};
use pensieve_server::agent::local::{
    NullCredentialStore, NullEnabledSkillsStore, NullEnginePreferenceStore,
};
use pensieve_server::agent::AgentState;
use pensieve_server::auth::{
    require_role_middleware, AuthBackend, AuthLayerState, EnvAuthBackend, Role,
};
use serde_json::{json, Value};
use tower::ServiceExt;

#[derive(Debug)]
struct MockEmbed;

#[async_trait::async_trait]
impl pensieve_embed::EmbeddingBackend for MockEmbed {
    fn id(&self) -> &str {
        "mock/source-summary"
    }
    fn dimension(&self) -> u16 {
        4
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, pensieve_embed::EmbedError> {
        Ok(texts
            .iter()
            .map(|t| vec![t.len() as f32, 1.0, 2.0, 3.0])
            .collect())
    }
}

type CatalogArc = Arc<dyn pensieve_core::catalog::Catalog>;
type FormatArc = Arc<dyn pensieve_core::segment_format::SegmentFormat>;

/// Embedded engine: in-memory SQLite catalog + local-fs segment format.
async fn embedded_engine(root: &std::path::Path) -> (CatalogArc, FormatArc) {
    let catalog: CatalogArc = Arc::new(
        pensieve_catalog_sqlite::SqliteCatalog::connect_in_memory()
            .await
            .expect("in-memory catalog"),
    );
    let store = pensieve_storage::build_object_store(&pensieve_storage::StorageConfig::Local {
        root: root.to_string_lossy().to_string(),
    })
    .expect("local store");
    let format: FormatArc = Arc::new(pensieve_format_tlm::TelemetryFormat::new(store, "test"));
    (catalog, format)
}

/// Agent router wrapped in the same `Role::Read` middleware production uses.
fn agent_app(catalog: CatalogArc, format: FormatArc) -> axum::Router {
    let agent_state = AgentState {
        catalog,
        format,
        pool: None,
        engines: Arc::new(NullEnginePreferenceStore),
        credentials: Arc::new(NullCredentialStore),
        tenant: DEFAULT_TENANT,
        skills: Arc::new(NullEnabledSkillsStore),
        mcp_url: None,
        memory: None,
        local_dreaming: None,
        memory_settings_path: None,
        consumer_events: None,
    };
    let backend: Arc<dyn AuthBackend> = Arc::new(EnvAuthBackend::from_str("read-token:read"));
    pensieve_server::agent::router(agent_state).layer(axum::middleware::from_fn_with_state(
        AuthLayerState {
            backend,
            required: Role::Read,
        },
        require_role_middleware,
    ))
}

fn memory(content: &str, realm: &str, provenance: Option<Value>) -> CreateMemory {
    let mut m = CreateMemory::new(content);
    m.realm = realm.to_string();
    m.provenance = provenance;
    m
}

#[tokio::test]
async fn source_summary_groups_by_provenance_source_and_realm() {
    let tmp = tempfile::tempdir().expect("tmp dir");
    let (catalog, format) = embedded_engine(tmp.path()).await;
    let writer = MemoryWriter::new(catalog.clone(), format.clone(), Arc::new(MockEmbed));

    // Two claude-code memories in realm "pensieve" (distinct provenance blobs —
    // the per-blob SQL groups must merge by parsed source).
    for run in ["r1", "r2"] {
        writer
            .save(&memory(
                &format!("cc memory {run}"),
                "pensieve",
                Some(json!({"source": "claude-code", "run_id": run})),
            ))
            .await
            .expect("save claude-code memory");
    }
    // One memory with no provenance → reported as "manual".
    writer
        .save(&memory("manual memory", "default", None))
        .await
        .expect("save manual memory");
    // One dreaming memory whose LATEST version is archived → excluded entirely.
    let dreamed = memory(
        "dreamed memory",
        "pensieve",
        Some(json!({"source": "dreaming"})),
    );
    let id = writer.save(&dreamed).await.expect("save dreaming memory");
    let emb = writer.embed_one(&dreamed.content).await.expect("embed");
    let mut archived = pensieve_memory::rows::node_row(&id, &dreamed, &emb, "2999-01-01T00:00:00Z");
    archived["status"] = json!("archived");
    writer
        .append_node_rows(vec![archived])
        .await
        .expect("archive dreaming memory");

    let req = Request::builder()
        .uri("/memory/source-summary")
        .header("authorization", "Bearer read-token")
        .body(Body::empty())
        .unwrap();
    let res = agent_app(catalog, format).oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(
        body["items"],
        json!([
            {"source": "claude-code", "realm": "pensieve", "count": 2},
            {"source": "manual", "realm": "default", "count": 1},
        ]),
        "expected claude-code/pensieve=2 + manual/default=1, archived dreaming excluded; got {body}",
    );
}
