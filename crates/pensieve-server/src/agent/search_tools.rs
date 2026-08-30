//! Agent tools over the unified `/v1/search` substrate — `search` (data mode)
//! and `graph_search` (graph mode).
//!
//! These give MCP-driven coding agents the *same* hybrid lexical+vector data
//! search and cross-graph node search that backs `POST /v1/search` (and the
//! Explore UI), through one in-process call to
//! [`crate::search::unified::unified_search`]. No new retrieval logic lives
//! here — each tool builds a [`UnifiedSearchRequest`], runs it through the
//! shared dispatcher, and returns the [`UnifiedSearchResponse`] as JSON (or a
//! `{"error": …}` payload on the dispatcher's `Err(Response)` arm, so a tool
//! failure is surfaced to the model as data rather than aborting the run).
//!
//! The memory mode of the substrate is intentionally **not** re-exposed here:
//! the `memory_search` / `recall_memory` tools already call the same
//! `retrieve()` function `unified_search(Memory)` delegates to, and they return
//! a strictly richer payload (distance, kw_score, graph_proximity, importance,
//! validity, via) than the lossier `UnifiedHit`. They share the substrate
//! without going through this lossy envelope.
//!
//! Agent-agnostic: nothing here names a specific coding agent. The MCP
//! dispatch ([`pensieve_mcp`]) registers these per its per-agent-kind registry.

use std::sync::Arc;

use adk_rust::tool::FunctionTool;
use adk_rust::{Tool, ToolContext};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::tools::SharedToolCtx;
use crate::discover::scope::Scope;
use crate::search::types::SearchMode;
use crate::search::unified::{unified_search, SearchCtx, UnifiedSearchRequest};

/// Build a [`SearchCtx`] from the MCP/agent [`SharedToolCtx`].
///
/// Agents are tenant-scoped at the transport/auth layer, so here we resolve
/// against `DEFAULT_TENANT` with no RBAC allow-list — the same defaults
/// [`SearchCtx::from_query_state`] applies when a request carries no scoped
/// principal (`tenant: DEFAULT_TENANT`, `allowed_databases: None`, where
/// `None` means "no restriction / all resolved sources pass"). `node_id` is
/// `None`, matching how the other agent tools build their `PensieveTable`s (see
/// [`super::tools::execute_sql`], which uses `PensieveTable::new` without a node
/// id). `pool` is wrapped into the `Arc` shape `SearchCtx` expects.
fn search_ctx_from_shared(shared: &SharedToolCtx) -> SearchCtx {
    SearchCtx {
        catalog: shared.catalog.clone(),
        format: shared.format.clone(),
        node_id: None,
        pool: shared.pool.clone().map(Arc::new),
        tenant: pensieve_core::tenant::DEFAULT_TENANT,
        allowed_databases: None,
    }
}

/// Run a [`UnifiedSearchRequest`] through the shared substrate and render the
/// envelope as JSON. The dispatcher's `Err(axum::Response)` arm (e.g. a bad
/// scope / time range) becomes a `{"error": …}` payload so the model can
/// self-correct instead of the run aborting.
async fn run_unified(shared: &SharedToolCtx, req: UnifiedSearchRequest, request_id: &str) -> Value {
    let ctx = search_ctx_from_shared(shared);
    match unified_search(&ctx, req, request_id).await {
        Ok(resp) => serde_json::to_value(&resp)
            .unwrap_or_else(|e| json!({"error": format!("serialize: {e}")})),
        // The Err arm is an HTTP `Response`; extract its status so the model
        // gets an actionable message rather than an opaque blob.
        Err(resp) => json!({
            "error": format!("search failed: HTTP {}", resp.status().as_u16()),
        }),
    }
}

// ---------------------------------------------------------------------------
// search — data mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct SearchArgs {
    /// Natural-language / keyword query. Embedded server-side for the vector
    /// leg; also drives the lexical (token-index `contains`) leg.
    query: String,
    /// Optional scope: a `{ "kind": "sources", "sources": ["db.*", "db.table",
    /// …] }` object of `db.table` glob patterns (or `{ "kind": "all" }`). Omit
    /// to search every data source. Taken as raw JSON so the tool needn't
    /// derive schema traits on the internal `Scope` type; an unparseable scope
    /// is reported as an `args` error.
    #[serde(default)]
    scope: Option<Value>,
    /// Max hits to return (default 50, server-capped at 500).
    #[serde(default)]
    limit: Option<usize>,
}

/// Parse the raw `scope` JSON into the internal [`Scope`], if present.
fn parse_scope(raw: Option<Value>) -> Result<Option<Scope>, String> {
    match raw {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| format!("scope: {e}")),
    }
}

const SEARCH_DESC: &str = "Hybrid lexical+vector search across data sources \
(the shared /v1/search substrate, data mode). One broad query fans out over \
every source in scope, running a keyword leg and (where a vector column \
exists) a semantic leg, fused by RRF. Returns ranked hits carrying `db.table` \
provenance + the matched row — follow up with run_kql / run_sql to correlate. \
Scope with `{\"kind\":\"sources\",\"sources\":[\"db.*\"]}` or omit for all sources.";

pub fn tool_search(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "search",
            SEARCH_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: SearchArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let scope = match parse_scope(parsed.scope) {
                        Ok(s) => s,
                        Err(e) => return Ok(json!({"error": e})),
                    };
                    let req = UnifiedSearchRequest {
                        query: parsed.query,
                        mode: SearchMode::Data,
                        scope,
                        limit: parsed.limit,
                        ..Default::default()
                    };
                    Ok(run_unified(&shared, req, "mcp-search").await)
                }
            },
        )
        .with_parameters_schema::<SearchArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

// ---------------------------------------------------------------------------
// graph_search — graph mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct GraphSearchArgs {
    /// Text to match graph nodes against (by name/label/properties).
    query: String,
    /// Restrict to one named graph (e.g. "kg"). Omit to search every graph
    /// across all databases — Pensieve's namespaces are composite (db/graph), so a
    /// bare name can resolve in more than one database.
    #[serde(default)]
    graph: Option<String>,
    /// Restrict to nodes carrying any of these labels (e.g. ["Service"]).
    #[serde(default)]
    labels: Option<Vec<String>>,
    /// Max node hits to return (default 50, server-capped at 500).
    #[serde(default)]
    limit: Option<usize>,
}

const GRAPH_SEARCH_DESC: &str = "Search graph nodes by text/label across one \
or all graphs (shared substrate, graph mode). Pass `graph` to target one \
named graph, or omit it to search every graph across all databases; `labels` \
filters by node label. Returns ranked nodes with `<db>/<graph>` provenance, \
the node id, and a title — follow node ids with graph_traverse for a deeper \
subgraph.";

pub fn tool_graph_search(ctx: SharedToolCtx) -> Arc<dyn Tool> {
    let shared = ctx;
    Arc::new(
        FunctionTool::new(
            "graph_search",
            GRAPH_SEARCH_DESC,
            move |_tc: Arc<dyn ToolContext>, args: Value| {
                let shared = shared.clone();
                async move {
                    let parsed: GraphSearchArgs = match serde_json::from_value(args) {
                        Ok(v) => v,
                        Err(e) => return Ok(json!({"error": format!("args: {e}")})),
                    };
                    let req = UnifiedSearchRequest {
                        query: parsed.query,
                        mode: SearchMode::Graph,
                        graph: parsed.graph,
                        labels: parsed.labels,
                        limit: parsed.limit,
                        ..Default::default()
                    };
                    Ok(run_unified(&shared, req, "mcp-graph-search").await)
                }
            },
        )
        .with_parameters_schema::<GraphSearchArgs>()
        .with_read_only(true)
        .with_concurrency_safe(true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_rust::tool::SimpleToolContext;
    use std::sync::Arc;

    /// In-memory SqliteCatalog + local-store TelemetryFormat harness, mirroring
    /// the `unified.rs` smoke-test setup. No tenant/principal — the agent
    /// defaults (`DEFAULT_TENANT`, `allowed_databases: None`) apply.
    async fn empty_shared() -> SharedToolCtx {
        use pensieve_core::segment_format::SegmentFormat;
        let catalog: Arc<dyn pensieve_core::catalog::Catalog> = Arc::new(
            pensieve_catalog_sqlite::SqliteCatalog::connect_in_memory()
                .await
                .expect("in-memory catalog"),
        );
        let tmp = std::env::temp_dir().join(format!("pensieve-searchtool-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = pensieve_storage::build_object_store(&pensieve_storage::StorageConfig::Local {
            root: tmp.to_string_lossy().to_string(),
        })
        .unwrap();
        let format: Arc<dyn SegmentFormat> =
            Arc::new(pensieve_format_tlm::TelemetryFormat::new(store, "test"));
        SharedToolCtx {
            realm_scope: Default::default(),
            consumer_sink: None,
            federation: None,
            catalog,
            format,
            pool: None,
            memory: None,
            // read-only test ctx — no HITL approval gate.
            hitl: None,
            memory_settings_path: None,
        }
    }

    #[test]
    fn search_ctx_uses_agent_defaults() {
        // The ctx-builder maps catalog/format/pool through and applies the
        // agent defaults: DEFAULT_TENANT, no allow-list (all sources pass),
        // no node id.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let shared = rt.block_on(empty_shared());
        let ctx = search_ctx_from_shared(&shared);
        assert_eq!(ctx.tenant, pensieve_core::tenant::DEFAULT_TENANT);
        assert!(
            ctx.allowed_databases.is_none(),
            "no RBAC allow-list ⇒ all pass"
        );
        assert!(ctx.node_id.is_none());
        assert!(ctx.pool.is_none());
    }

    #[tokio::test]
    async fn search_tool_constructs_with_name_desc_schema() {
        let shared = empty_shared().await;
        let tool = tool_search(shared);
        assert_eq!(tool.name(), "search");
        assert!(tool.description().contains("data mode"));
        // A parameters schema is exposed for MCP `tools/list`.
        assert!(tool.parameters_schema().is_some());
    }

    #[tokio::test]
    async fn graph_search_tool_constructs_with_name_desc_schema() {
        let shared = empty_shared().await;
        let tool = tool_graph_search(shared);
        assert_eq!(tool.name(), "graph_search");
        assert!(tool.description().contains("graph mode"));
        assert!(tool.parameters_schema().is_some());
    }

    #[tokio::test]
    async fn search_returns_wellformed_empty_result_over_empty_catalog() {
        // No databases ⇒ no sources ⇒ a well-formed, empty data-mode envelope,
        // never a panic / error. (Data mode omits the `mode` tag for legacy
        // byte-compat, so we assert on the structural keys instead.)
        let shared = empty_shared().await;
        let tool = tool_search(shared);
        let tc = Arc::new(SimpleToolContext::new("test"));
        let out = tool
            .execute(tc, json!({ "query": "anything", "limit": 5 }))
            .await
            .expect("tool runs");
        assert!(out.get("error").is_none(), "no error: {out}");
        assert_eq!(out["hits"], json!([]));
        assert_eq!(out["sources_searched"], json!(0));
        assert!(out.get("elapsed_ms").is_some());
    }

    #[tokio::test]
    async fn graph_search_returns_wellformed_empty_result_over_empty_catalog() {
        // No graphs ⇒ an empty, graph-mode-tagged envelope, no panic.
        let shared = empty_shared().await;
        let tool = tool_graph_search(shared);
        let tc = Arc::new(SimpleToolContext::new("test"));
        let out = tool
            .execute(tc, json!({ "query": "anything" }))
            .await
            .expect("tool runs");
        assert!(out.get("error").is_none(), "no error: {out}");
        assert_eq!(out["hits"], json!([]));
        assert_eq!(out["sources_searched"], json!(0));
        assert_eq!(out["mode"], json!("graph"));
    }

    #[tokio::test]
    async fn graph_search_seeded_graph_returns_node_hits() {
        // End-to-end over a real seeded stored graph (pure SQL path, no
        // embedder needed): ingest a node table, register a graph, and assert
        // the tool surfaces a matching node hit through the substrate.
        use pensieve_core::catalog::{GraphSpec, TableConfig};
        let shared = empty_shared().await;
        let catalog = shared.catalog.clone();
        let format = shared.format.clone();

        let db_id = catalog.create_database("kg").await.expect("create db kg");
        let schema = Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("id", arrow_schema::DataType::Utf8, false),
            arrow_schema::Field::new("labels", arrow_schema::DataType::Utf8, true),
            arrow_schema::Field::new("name", arrow_schema::DataType::Utf8, true),
            arrow_schema::Field::new("realm", arrow_schema::DataType::Utf8, true),
        ]));
        catalog
            .create_table(db_id, "kg_nodes", schema.clone(), TableConfig::default())
            .await
            .expect("create kg_nodes");
        catalog
            .create_table(db_id, "kg_edges", schema.clone(), TableConfig::default())
            .await
            .expect("create kg_edges");

        let tref = catalog
            .lookup_table("kg", "kg_nodes")
            .await
            .expect("lookup kg_nodes");
        let ndjson = r#"{"id":"svc:alpha","labels":"Service","name":"alpha-service","realm":"kg"}
{"id":"svc:beta","labels":"Service","name":"beta-service","realm":"kg"}"#;
        let batches = pensieve_ingest_core::parse_ndjson(ndjson.as_bytes(), tref.schema.clone())
            .expect("parse ndjson");
        pensieve_ingest_core::WritePath::new(catalog.clone(), format.clone())
            .ingest("kg", &tref, batches)
            .await
            .expect("ingest node rows");

        let mut spec = GraphSpec::with_defaults("kg_nodes", "kg_edges");
        spec.realm_col = Some("realm".into());
        catalog
            .create_graph("kg", "kg", spec)
            .await
            .expect("create_graph kg");

        let tool = tool_graph_search(shared);
        let tc = Arc::new(SimpleToolContext::new("test"));
        let out = tool
            .execute(tc, json!({ "query": "alpha", "graph": "kg" }))
            .await
            .expect("tool runs");

        assert!(out.get("error").is_none(), "no error: {out}");
        assert_eq!(out["mode"], json!("graph"));
        let hits = out["hits"].as_array().expect("hits array");
        assert!(
            hits.iter().any(|h| h["id"] == json!("svc:alpha")),
            "expected svc:alpha node hit, got {out}"
        );
        let alpha = hits.iter().find(|h| h["id"] == json!("svc:alpha")).unwrap();
        assert_eq!(alpha["kind"], json!("node"));
        assert_eq!(alpha["source"], json!("kg/kg"));
        assert_eq!(alpha["title"], json!("alpha-service"));
    }
}
