//! HTTP surface for the graph layer (`/v1/graph/*`). G1a serves only the
//! synthetic `"schema"` graph (catalog rendered as a property-graph).

use std::sync::Arc;

use async_trait::async_trait;
use kyma_core::catalog::Catalog;
use kyma_graph::{ColumnDef, SchemaSource};

/// Adapts the full [`Catalog`] down to the narrow [`SchemaSource`] the
/// schema-graph needs.
pub struct CatalogSchemaSource {
    catalog: Arc<dyn Catalog>,
}

impl CatalogSchemaSource {
    pub fn new(catalog: Arc<dyn Catalog>) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl SchemaSource for CatalogSchemaSource {
    async fn databases(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.catalog.list_databases().await.map_err(anyhow::Error::from)?)
    }
    async fn tables(&self, database: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.catalog.list_tables(database).await.map_err(anyhow::Error::from)?)
    }
    async fn columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnDef>> {
        let cols = self
            .catalog
            .get_table_columns(database, table)
            .await
            .map_err(anyhow::Error::from)?;
        Ok(cols
            .into_iter()
            .map(|c| ColumnDef { name: c.name, type_: c.r#type, nullable: c.nullable })
            .collect())
    }
}

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use kyma_graph::{Direction, GraphProvider, GraphRef, SchemaGraphProvider};
use serde::Deserialize;

use crate::QueryState;

const SCHEMA_GRAPH: &str = "schema";

/// Build a `SchemaGraphProvider` over the request's catalog.
fn provider(state: &QueryState) -> SchemaGraphProvider {
    SchemaGraphProvider::new(Arc::new(CatalogSchemaSource::new(state.catalog.clone())))
}

/// Map any provider error to a 500 JSON envelope (consistent with other handlers).
fn err500(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": {"code": "graph", "message": e.to_string()}})),
    )
        .into_response()
}

/// 404 unless the graph name is the synthetic schema-graph (G1a).
fn ensure_schema(graph: &str) -> Result<(), Response> {
    if graph == SCHEMA_GRAPH {
        Ok(())
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": {"code": "not_found", "message": "unknown graph"}})),
        )
            .into_response())
    }
}

#[derive(Deserialize)]
struct OverviewQuery {
    realm: Option<String>,
    #[serde(default = "default_overview_limit")]
    limit: usize,
}
fn default_overview_limit() -> usize {
    800
}

#[derive(Deserialize)]
struct RealmQuery {
    realm: Option<String>,
}

#[derive(Deserialize)]
struct SubgraphQuery {
    #[serde(default = "default_depth")]
    depth: usize,
}
fn default_depth() -> usize {
    2
}

#[derive(Deserialize)]
struct SearchBody {
    text: String,
    #[serde(default)]
    labels: Vec<String>,
    realm: Option<String>,
    #[serde(default = "default_search_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}
fn default_search_limit() -> usize {
    20
}

#[derive(Deserialize)]
struct NeighborsBody {
    node_ids: Vec<String>,
    #[serde(default = "default_direction")]
    direction: Direction,
    #[serde(default)]
    only_internal: bool,
    #[serde(default = "default_neighbors_limit")]
    limit: usize,
}
fn default_direction() -> Direction {
    Direction::Both
}
fn default_neighbors_limit() -> usize {
    200
}

async fn list_graphs(State(_state): State<QueryState>) -> Response {
    let refs = vec![GraphRef {
        name: SCHEMA_GRAPH.into(),
        kind: "schema".into(),
        description: "Catalog schema as a property-graph (tables + inferred references).".into(),
    }];
    (StatusCode::OK, Json(refs)).into_response()
}

async fn overview(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Query(q): Query<OverviewQuery>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).overview(q.realm.as_deref(), q.limit).await {
        Ok(p) => (StatusCode::OK, Json(p)).into_response(),
        Err(e) => err500(e),
    }
}

async fn stats(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Query(q): Query<RealmQuery>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).stats(q.realm.as_deref()).await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => err500(e),
    }
}

async fn schema(State(state): State<QueryState>, Path(graph): Path<String>) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).schema().await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => err500(e),
    }
}

async fn node(
    State(state): State<QueryState>,
    Path((graph, id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).node(&id).await {
        Ok(Some(n)) => (StatusCode::OK, Json(n)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": {"code": "not_found", "message": "no such node"}})),
        )
            .into_response(),
        Err(e) => err500(e),
    }
}

async fn subgraph(
    State(state): State<QueryState>,
    Path((graph, id)): Path<(String, String)>,
    Query(q): Query<SubgraphQuery>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).subgraph(&id, q.depth).await {
        Ok(p) => (StatusCode::OK, Json(p)).into_response(),
        Err(e) => err500(e),
    }
}

async fn search(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Json(body): Json<SearchBody>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state)
        .search(&body.text, &body.labels, body.realm.as_deref(), body.limit, body.offset)
        .await
    {
        Ok(h) => (StatusCode::OK, Json(h)).into_response(),
        Err(e) => err500(e),
    }
}

async fn neighbors(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Json(body): Json<NeighborsBody>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state)
        .neighbors(&body.node_ids, body.direction, body.only_internal, body.limit)
        .await
    {
        Ok(x) => (StatusCode::OK, Json(x)).into_response(),
        Err(e) => err500(e),
    }
}

/// Read-only graph router. Caller wraps with the same `Role::Read` middleware
/// as the rest of the query surface.
pub fn graph_router(state: QueryState) -> Router {
    Router::new()
        .route("/v1/graph", get(list_graphs))
        .route("/v1/graph/:graph/overview", get(overview))
        .route("/v1/graph/:graph/stats", get(stats))
        .route("/v1/graph/:graph/schema", get(schema))
        .route("/v1/graph/:graph/nodes/:id", get(node))
        .route("/v1/graph/:graph/nodes/:id/subgraph", get(subgraph))
        .route("/v1/graph/:graph/search", post(search))
        .route("/v1/graph/:graph/neighbors", post(neighbors))
        .with_state(state)
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::*;
    use kyma_graph::{GraphProvider, SchemaGraphProvider};

    #[tokio::test]
    async fn adapter_feeds_schema_provider_from_seeded_catalog() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let source = Arc::new(CatalogSchemaSource::new(state.catalog.clone()));
        let provider = SchemaGraphProvider::new(source);
        let payload = provider.overview(None, 1000).await.unwrap();
        assert!(
            payload.nodes.iter().any(|n| n.id.ends_with("::otel_logs")),
            "expected an otel_logs table node, got: {:?}",
            payload.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
    }

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    #[tokio::test]
    async fn overview_endpoint_returns_schema_graph_json() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/graph/schema/overview?limit=500")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["stats"]["total_nodes"].as_u64().unwrap() >= 1);
        assert!(v["nodes"].as_array().unwrap().iter().any(|n| n["id"]
            .as_str()
            .unwrap()
            .ends_with("::otel_logs")));
    }

    #[tokio::test]
    async fn unknown_graph_name_is_404() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/graph/nope/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_endpoint_lists_schema_graph() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(Request::builder().uri("/v1/graph").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v[0]["name"], "schema");
        assert_eq!(v[0]["kind"], "schema");
    }

    #[tokio::test]
    async fn search_endpoint_returns_hits() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/graph/schema/search")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"text":"otel"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["total"].as_u64().unwrap() >= 1, "expected a hit for 'otel'");
        assert!(v["hits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["id"].as_str().unwrap().ends_with("::otel_logs")));
    }

    #[tokio::test]
    async fn neighbors_endpoint_ok_shape_with_default_direction() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        // direction omitted -> exercises the serde default (Both); node id need not exist.
        let res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/graph/schema/neighbors")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"node_ids":["default::otel_logs"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["edges"].is_array());
        assert!(v["new_node_ids"].is_array());
    }

    #[tokio::test]
    async fn node_endpoint_404_for_missing() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/graph/schema/nodes/default::does_not_exist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn graph_registration_crud_roundtrip() {
        use kyma_core::catalog::GraphSpec;
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let cat = &state.catalog;

        // none registered yet
        assert!(cat.list_graphs("obs").await.unwrap().is_empty());
        assert!(cat.get_graph("obs", "kg").await.unwrap().is_none());

        // create
        let mut spec = GraphSpec::with_defaults("kg_nodes", "kg_edges");
        spec.realm_col = Some("realm".into());
        let reg = cat.create_graph("obs", "kg", spec).await.unwrap();
        assert_eq!(reg.name, "kg");
        assert_eq!(reg.node_table, "kg_nodes");
        assert_eq!(reg.edge_table, "kg_edges");
        assert_eq!(reg.id_col, "id");
        assert_eq!(reg.realm_col.as_deref(), Some("realm"));

        // get + list
        let got = cat.get_graph("obs", "kg").await.unwrap().unwrap();
        assert_eq!(got.id, reg.id);
        assert_eq!(cat.list_graphs("obs").await.unwrap().len(), 1);

        // drop
        assert!(cat.drop_graph("obs", "kg").await.unwrap());
        assert!(!cat.drop_graph("obs", "kg").await.unwrap()); // idempotent: now false
        assert!(cat.get_graph("obs", "kg").await.unwrap().is_none());
    }
}
