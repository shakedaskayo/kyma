//! HTTP surface for the graph layer (`/v1/graph/*`). G1a serves the synthetic
//! `"schema"` graph (catalog rendered as a property-graph). G1b.2 adds routing
//! to registered stored graphs via `StoredGraphProvider`.

use std::sync::Arc;

use async_trait::async_trait;
use kyma_core::catalog::Catalog;
use kyma_graph::{ColumnDef, SchemaSource};

/// Adapts the full [`Catalog`] down to the narrow [`SchemaSource`] the
/// schema-graph needs. When `allowed` is `Some`, only those databases are
/// visible — this is how per-database token scope applies to the synthetic
/// schema graph (mirrors `catalog_handler::filter_schema_by_principal`).
pub struct CatalogSchemaSource {
    catalog: Arc<dyn Catalog>,
    allowed: Option<Vec<String>>,
}

impl CatalogSchemaSource {
    pub fn new(catalog: Arc<dyn Catalog>) -> Self {
        Self { catalog, allowed: None }
    }

    /// Restrict visible databases to `allowed` (None = unrestricted).
    pub fn with_allowed(catalog: Arc<dyn Catalog>, allowed: Option<Vec<String>>) -> Self {
        Self { catalog, allowed }
    }
}

#[async_trait]
impl SchemaSource for CatalogSchemaSource {
    async fn databases(&self) -> anyhow::Result<Vec<String>> {
        let mut dbs = self.catalog.list_databases().await.map_err(anyhow::Error::from)?;
        if let Some(allowed) = &self.allowed {
            dbs.retain(|d| allowed.iter().any(|a| a == d));
        }
        Ok(dbs)
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

// ---------------------------------------------------------------------------
// QueryEngineExecutor: runs SQL against a database via DataFusion + KymaTable.
// Mirrors agent/tools.rs::execute_sql exactly.
// ---------------------------------------------------------------------------

use arrow::json::ArrayWriter;
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::prelude::{SessionConfig, SessionContext};
use kyma_exec::KymaTable;
use kyma_graph::{GraphQueryExecutor, JsonRow};

const GRAPH_MEMORY_POOL_BYTES: usize = 256 * 1024 * 1024;

pub(crate) struct QueryEngineExecutor {
    catalog: Arc<dyn kyma_core::catalog::Catalog>,
    format: Arc<dyn kyma_core::segment_format::SegmentFormat>,
}

#[async_trait]
impl GraphQueryExecutor for QueryEngineExecutor {
    async fn query(&self, database: &str, sql: String) -> anyhow::Result<Vec<JsonRow>> {
        let tables = self
            .catalog
            .list_tables_in_database(database)
            .await
            .map_err(|e| anyhow::anyhow!("list_tables_in_database({database}): {e}"))?;

        let runtime = RuntimeEnvBuilder::new()
            .with_memory_pool(Arc::new(GreedyMemoryPool::new(GRAPH_MEMORY_POOL_BYTES)))
            .build()
            .map(Arc::new)
            .map_err(|e| anyhow::anyhow!("runtime_env: {e}"))?;

        let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
        kyma_exec::register_vector_udfs(&ctx);

        for t in tables {
            let name = t.name.clone();
            let table = Arc::new(KymaTable::new(t, self.catalog.clone(), self.format.clone()));
            ctx.register_table(&name, table)
                .map_err(|e| anyhow::anyhow!("register_table({name}): {e}"))?;
        }

        let batches = ctx
            .sql(&sql)
            .await
            .map_err(|e| anyhow::anyhow!("sql_plan: {e}"))?
            .collect()
            .await
            .map_err(|e| anyhow::anyhow!("sql_exec: {e}"))?;

        // Arrow→JSON using the same ArrayWriter pattern as agent/tools.rs::execute_sql.
        let mut rows: Vec<JsonRow> = Vec::new();
        for batch in &batches {
            let mut buf: Vec<u8> = Vec::with_capacity(batch.num_rows() * 128);
            {
                let mut writer = ArrayWriter::new(&mut buf);
                writer.write(batch).map_err(|e| anyhow::anyhow!("serialize: {e}"))?;
                writer.finish().map_err(|e| anyhow::anyhow!("serialize_finish: {e}"))?;
            }
            let parsed: serde_json::Value = serde_json::from_slice(&buf)
                .map_err(|e| anyhow::anyhow!("reparse: {e}"))?;
            if let serde_json::Value::Array(arr) = parsed {
                for row in arr {
                    if let serde_json::Value::Object(map) = row {
                        rows.push(map);
                    }
                }
            }
        }
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Routing helpers
// ---------------------------------------------------------------------------

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use kyma_graph::{Direction, GraphProvider, GraphRef, SchemaGraphProvider, StoredGraphProvider};
use serde::Deserialize;

use crate::QueryState;

const SCHEMA_GRAPH: &str = "schema";

/// Map any provider error to a 500 JSON envelope (consistent with other handlers).
fn err500(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": {"code": "graph", "message": e.to_string()}})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// ResolvedProvider: schema OR stored, unified behind GraphProvider.
// ---------------------------------------------------------------------------

pub(crate) enum ResolvedProvider {
    Schema(SchemaGraphProvider),
    Stored(StoredGraphProvider),
}

#[async_trait]
impl GraphProvider for ResolvedProvider {
    async fn overview(
        &self,
        realm: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<kyma_graph::GraphPayload> {
        match self {
            Self::Schema(p) => p.overview(realm, limit).await,
            Self::Stored(p) => p.overview(realm, limit).await,
        }
    }
    async fn node(&self, id: &str) -> anyhow::Result<Option<kyma_graph::GraphNode>> {
        match self {
            Self::Schema(p) => p.node(id).await,
            Self::Stored(p) => p.node(id).await,
        }
    }
    async fn neighbors(
        &self,
        ids: &[String],
        dir: Direction,
        only_internal: bool,
        limit: usize,
    ) -> anyhow::Result<kyma_graph::EdgeExpansion> {
        match self {
            Self::Schema(p) => p.neighbors(ids, dir, only_internal, limit).await,
            Self::Stored(p) => p.neighbors(ids, dir, only_internal, limit).await,
        }
    }
    async fn subgraph(
        &self,
        id: &str,
        depth: usize,
    ) -> anyhow::Result<kyma_graph::GraphPayload> {
        match self {
            Self::Schema(p) => p.subgraph(id, depth).await,
            Self::Stored(p) => p.subgraph(id, depth).await,
        }
    }
    async fn search(
        &self,
        text: &str,
        labels: &[String],
        realm: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<kyma_graph::SearchHits> {
        match self {
            Self::Schema(p) => p.search(text, labels, realm, limit, offset).await,
            Self::Stored(p) => p.search(text, labels, realm, limit, offset).await,
        }
    }
    async fn stats(&self, realm: Option<&str>) -> anyhow::Result<kyma_graph::GraphStats> {
        match self {
            Self::Schema(p) => p.stats(realm).await,
            Self::Stored(p) => p.stats(realm).await,
        }
    }
    async fn schema(&self) -> anyhow::Result<kyma_graph::GraphSchema> {
        match self {
            Self::Schema(p) => p.schema().await,
            Self::Stored(p) => p.schema().await,
        }
    }
}

/// Resolve a graph name + database to either the synthetic schema-graph or a
/// registered stored graph.  Returns a 404 or 500 `Response` on failure.
///
/// Thin wrapper over [`resolve_with`] that pulls the catalog/format out of the
/// HTTP handler state and resolves under the default tenant (the graph HTTP
/// surface is keyed by the `x-database` header + per-database token scope, not
/// by a tenant id). The unified-search Graph arm calls [`resolve_with`]
/// directly with the request's tenant.
async fn resolve(
    state: &QueryState,
    graph: &str,
    database: &str,
    allowed_databases: Option<Vec<String>>,
) -> Result<ResolvedProvider, Response> {
    resolve_with(
        &state.catalog,
        &state.format,
        kyma_core::tenant::DEFAULT_TENANT,
        graph,
        database,
        allowed_databases,
    )
    .await
}

/// Catalog/format-level graph resolver shared by the `/v1/graph/*` HTTP
/// handlers and the unified-search Graph arm. Takes the two `Arc` handles plus
/// the resolving `tenant` directly so callers that don't carry a full
/// [`QueryState`] (e.g. `search::unified`) can reuse the exact same
/// schema-vs-stored routing + `StoredGraphProvider` wiring instead of
/// duplicating it. Returns a 404 / 500 `Response` on failure.
pub(crate) async fn resolve_with(
    catalog: &Arc<dyn kyma_core::catalog::Catalog>,
    format: &Arc<dyn kyma_core::segment_format::SegmentFormat>,
    tenant: kyma_core::tenant::TenantId,
    graph: &str,
    database: &str,
    allowed_databases: Option<Vec<String>>,
) -> Result<ResolvedProvider, Response> {
    if graph == SCHEMA_GRAPH {
        // The schema graph spans all databases — apply the principal's
        // per-database scope so scoped tokens can't enumerate metadata of
        // databases outside their allow-list.
        return Ok(ResolvedProvider::Schema(SchemaGraphProvider::new(Arc::new(
            CatalogSchemaSource::with_allowed(catalog.clone(), allowed_databases),
        ))));
    }
    match catalog.get_graph_in_tenant(tenant, database, graph).await {
        Ok(Some(reg)) => Ok(ResolvedProvider::Stored(stored_provider(catalog, format, reg))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(
                serde_json::json!({"error": {"code": "not_found", "message": "unknown graph"}}),
            ),
        )
            .into_response()),
        Err(e) => Err(err500(anyhow::anyhow!(e.to_string()))),
    }
}

/// Build a `StoredGraphProvider` over a registration, wiring the DataFusion
/// query executor from the catalog + format. The single construction site for
/// stored-graph providers (HTTP handlers + unified search both route here).
pub(crate) fn stored_provider(
    catalog: &Arc<dyn kyma_core::catalog::Catalog>,
    format: &Arc<dyn kyma_core::segment_format::SegmentFormat>,
    reg: kyma_core::catalog::GraphRegistration,
) -> StoredGraphProvider {
    let cfg = kyma_graph::StoredGraphConfig {
        database: reg.database,
        node_table: reg.node_table,
        edge_table: reg.edge_table,
        id_col: reg.id_col,
        label_col: reg.label_col,
        src_col: reg.src_col,
        dst_col: reg.dst_col,
        type_col: reg.type_col,
        realm_col: reg.realm_col,
    };
    let exec = Arc::new(QueryEngineExecutor {
        catalog: catalog.clone(),
        format: format.clone(),
    });
    StoredGraphProvider::new(cfg, exec)
}

// ---------------------------------------------------------------------------
// Query-param structs
// ---------------------------------------------------------------------------

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

#[derive(Deserialize)]
struct ExportQuery {
    realm: Option<String>,
    #[serde(default)]
    algorithm: kyma_graph::LayoutAlgorithm,
    cursor: Option<String>,
    #[serde(default = "default_export_page_size")]
    page_size: usize,
}
fn default_export_page_size() -> usize {
    crate::graph_layout_cache::PAGE_SIZE_DEFAULT
}

// ---------------------------------------------------------------------------
// Handlers — each reads x-database from headers and dispatches via resolve().
// ---------------------------------------------------------------------------

fn db_from_headers(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-database")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

/// Enforce per-database token scope. Returns `Err(Response)` on violation.
/// When `db` is empty (no x-database header), there is nothing to enforce.
fn enforce_scope(
    principal: Option<&crate::auth::Principal>,
    db: &str,
) -> Result<(), Response> {
    if db.is_empty() {
        return Ok(());
    }
    if let Some(p) = principal {
        if let Err((status, msg)) = crate::auth::check_database_scope(p, db) {
            return Err((
                status,
                Json(serde_json::json!({"error": {"code": "forbidden", "message": msg}})),
            )
                .into_response());
        }
    }
    Ok(())
}

async fn list_graphs(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let mut refs = vec![GraphRef {
        name: SCHEMA_GRAPH.into(),
        kind: "schema".into(),
        description: "Catalog schema as a property-graph (tables + inferred references).".into(),
    }];
    if !db.is_empty() {
        if let Ok(regs) = state.catalog.list_graphs(&db).await {
            for r in regs {
                refs.push(GraphRef {
                    name: r.name,
                    kind: "stored".into(),
                    description: format!("nodes={}, edges={}", r.node_table, r.edge_table),
                });
            }
        }
    }
    (StatusCode::OK, Json(refs)).into_response()
}

async fn overview(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path(graph): Path<String>,
    Query(q): Query<OverviewQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match p.overview(q.realm.as_deref(), q.limit).await {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => err500(e),
    }
}

async fn stats(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path(graph): Path<String>,
    Query(q): Query<RealmQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match p.stats(q.realm.as_deref()).await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => err500(e),
    }
}

async fn schema(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path(graph): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match p.schema().await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => err500(e),
    }
}

async fn node(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path((graph, id)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match p.node(&id).await {
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
    principal: Option<Extension<crate::auth::Principal>>,
    Path((graph, id)): Path<(String, String)>,
    Query(q): Query<SubgraphQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match p.subgraph(&id, q.depth).await {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => err500(e),
    }
}

async fn search(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path(graph): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<SearchBody>,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match p
        .search(&body.text, &body.labels, body.realm.as_deref(), body.limit, body.offset)
        .await
    {
        Ok(h) => (StatusCode::OK, Json(h)).into_response(),
        Err(e) => err500(e),
    }
}

async fn neighbors(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path(graph): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<NeighborsBody>,
) -> Response {
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    match p
        .neighbors(&body.node_ids, body.direction, body.only_internal, body.limit)
        .await
    {
        Ok(x) => (StatusCode::OK, Json(x)).into_response(),
        Err(e) => err500(e),
    }
}

/// Full-graph export with precomputed layout positions, paginated.
/// First call (no cursor): checks cache freshness via stats fingerprint,
/// computes layout (inline for small graphs, background task for large),
/// and returns the first node page or `layout_status: "computing"`.
/// Subsequent calls pass the returned cursor and are served from the cache.
async fn export(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path(graph): Path<String>,
    Query(q): Query<ExportQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    use crate::graph_layout_cache::{
        parse_cursor, slice_page, CacheKey, CacheState, LaidOutGraph, LayoutCache,
        SYNC_COMPUTE_MAX_NODES,
    };
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let page_size = q.page_size.clamp(100, 50_000);

    // Pages 2+: serve straight from the layout_id in the cursor.
    if let Some(cursor) = &q.cursor {
        let Some((layout_id, kind, offset)) = parse_cursor(cursor) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": {"code": "bad_cursor", "message": "malformed cursor"}})),
            )
                .into_response();
        };
        return match state.layout_cache.by_layout_id(&layout_id) {
            Some(laid) => {
                (StatusCode::OK, Json(slice_page(&laid, kind, offset, page_size))).into_response()
            }
            // Evicted mid-paging — client restarts from cursor=None.
            None => (
                StatusCode::GONE,
                Json(serde_json::json!({"error": {"code": "layout_evicted", "message": "layout evicted; restart export"}})),
            )
                .into_response(),
        };
    }

    // First page: resolve provider + fingerprint.
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let stats = match p.stats(q.realm.as_deref()).await {
        Ok(s) => s,
        Err(e) => return err500(e),
    };
    let fingerprint = (stats.total_nodes, stats.total_relationships);
    let key = CacheKey {
        database: db.clone(),
        graph: graph.clone(),
        realm: q.realm.clone(),
        algorithm: q.algorithm,
    };
    let computing = |key: &CacheKey, fp: (usize, usize)| {
        Json(kyma_graph::GraphExportPage {
            layout_status: "computing".into(),
            layout_id: LayoutCache::layout_id(key, fp),
            total_nodes: fp.0,
            total_edges: fp.1,
            nodes: vec![],
            edges: vec![],
            next_cursor: None,
        })
    };

    match state.layout_cache.get_fresh(&key, fingerprint) {
        Some(CacheState::Ready(laid)) => {
            (StatusCode::OK, Json(slice_page(&laid, 'n', 0, page_size))).into_response()
        }
        Some(CacheState::Computing) => {
            (StatusCode::OK, computing(&key, fingerprint)).into_response()
        }
        None => {
            let compute = move |payload: kyma_graph::GraphPayload,
                                key: &CacheKey|
                  -> LaidOutGraph {
                let positions = kyma_graph::compute_layout(
                    key.algorithm,
                    &payload.nodes,
                    &payload.edges,
                    kyma_graph::LAYOUT_WIDTH,
                    kyma_graph::LAYOUT_HEIGHT,
                );
                let fp = (payload.nodes.len(), payload.edges.len());
                LaidOutGraph {
                    layout_id: LayoutCache::layout_id(key, fp),
                    fingerprint: fp,
                    nodes: payload
                        .nodes
                        .into_iter()
                        .map(|n| {
                            let (x, y) = positions.get(&n.id).copied().unwrap_or((0.0, 0.0));
                            kyma_graph::PositionedNode { node: n, x, y }
                        })
                        .collect(),
                    edges: payload.edges,
                }
            };

            if stats.total_nodes <= SYNC_COMPUTE_MAX_NODES {
                // Small graph: compute inline and serve the first page now.
                let payload = match p.overview(q.realm.as_deref(), usize::MAX).await {
                    Ok(pl) => pl,
                    Err(e) => return err500(e),
                };
                let laid = state.layout_cache.insert_ready(key.clone(), compute(payload, &key));
                (StatusCode::OK, Json(slice_page(&laid, 'n', 0, page_size))).into_response()
            } else {
                // Large graph: background compute, report computing.
                if state.layout_cache.begin_compute(&key) {
                    let cache = state.layout_cache.clone();
                    let realm = q.realm.clone();
                    let bg_key = key.clone();
                    let mut guard = cache.compute_guard(bg_key.clone());
                    tokio::spawn(async move {
                        match p.overview(realm.as_deref(), usize::MAX).await {
                            Ok(payload) => {
                                let laid = compute(payload, &bg_key);
                                cache.insert_ready(bg_key, laid);
                                guard.disarm();
                            }
                            Err(e) => {
                                tracing::warn!("graph export layout failed: {e}");
                                // guard drops here, calling abort_compute
                            }
                        }
                    });
                }
                (StatusCode::OK, computing(&key, fingerprint)).into_response()
            }
        }
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
        .route("/v1/graph/:graph/export", get(export))
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

    /// Integration test: register a stored graph over otel_logs (reused as both
    /// node + edge table since any real table works for executor wiring). Verify:
    /// 1. GET /v1/graph with x-database:obs lists the registered graph (kind=stored).
    /// 2. GET /v1/graph/kg/stats with x-database:obs returns 200 (executor ran SQL).
    ///
    /// Column roles are mapped to columns that actually exist in otel_logs:
    ///   id_col="timestamp", label_col="severity_text", src_col="service.name",
    ///   dst_col="service.name", type_col="severity_text"
    /// so all SQL is valid even though the data has no real graph semantics.
    #[tokio::test]
    async fn stored_graph_routing_and_executor_wiring() {
        use kyma_core::catalog::GraphSpec;

        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let cat = &state.catalog;

        // Register "kg" over otel_logs with roles pointing to existing columns.
        let mut spec = GraphSpec::with_defaults("otel_logs", "otel_logs");
        spec.id_col = "timestamp".into();
        spec.label_col = "severity_text".into();
        spec.src_col = "service.name".into();
        spec.dst_col = "service.name".into();
        spec.type_col = "severity_text".into();
        spec.realm_col = None;
        cat.create_graph("obs", "kg", spec).await.unwrap();

        let app = graph_router(state);

        // 1. GET /v1/graph with x-database: obs → list includes schema + kg (stored)
        let res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/graph")
                    .header("x-database", "obs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let list: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let arr = list.as_array().unwrap();
        assert!(
            arr.iter().any(|g| g["name"] == "schema"),
            "schema entry missing: {list}"
        );
        assert!(
            arr.iter().any(|g| g["name"] == "kg" && g["kind"] == "stored"),
            "kg/stored entry missing: {list}"
        );

        // 2. GET /v1/graph/kg/stats with x-database: obs → 200 (executor ran SQL)
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/graph/kg/stats")
                    .header("x-database", "obs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "stats endpoint should return 200 for registered stored graph"
        );
    }
}
