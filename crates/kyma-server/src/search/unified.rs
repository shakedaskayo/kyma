//! Unified search substrate: one in-process dispatcher behind `POST /v1/search`.
//!
//! `unified_search` is the shared entry point the HTTP handler routes through.
//! It selects a backend by [`SearchMode`] and returns a single
//! [`UnifiedSearchResponse`] envelope across data/memory/graph modes.
//!
//! The **Data** arm reuses the existing per-source lexical+vector RRF fan-out
//! via [`super::search_data`] so the legacy data path and the unified path share
//! one code path. The **Memory** arm delegates to the agent's graph-aware hybrid
//! [`retrieve`](crate::agent::memory_retrieve::retrieve) so unified search and
//! the `recall_memory` / `memory_search` tools share one retrieval path and full
//! recall quality is preserved. The **Graph** arm delegates to the same graph
//! provider that backs `POST /v1/graph/<name>/search`: it resolves the
//! requested graph(s) through [`crate::graph_handler::resolve_with`] and calls
//! `GraphProvider::search`, so the unified path and the graph HTTP surface share
//! one resolution + search code path. It never fabricates data — an unknown or
//! unresolvable graph yields an empty (mode-echoed) envelope rather than a 500.
//!
//! ## Backward compatibility (Data mode)
//!
//! For `mode == Data` the envelope serializes byte-for-byte like the legacy
//! `SearchResponse`: `{ "hits": [{ "source", "score", "row" }], "sources_searched",
//! "elapsed_ms" }`. To guarantee that, the Data arm sets `kind: None` on every
//! hit (so the `"kind"` key is omitted via `skip_serializing_if`) and leaves
//! `mode`/`context`/`linked` as `None`. TS clients tolerate extra keys, but
//! omitting `kind` makes the legacy shape provably identical rather than merely
//! "ignored by current clients".

use std::sync::Arc;
use std::time::Instant;

use serde::Deserialize;
use serde_json::Value;

use kyma_graph::GraphProvider;

use super::types::{SearchMode, UnifiedHit, UnifiedSearchResponse};
use super::{search_data, DEFAULT_LIMIT, MAX_LIMIT};
use crate::agent::memory_retrieve::{retrieve, RetrieveRequest, RetrieveResult};
use crate::agent::tools::SharedToolCtx;
use crate::discover::compile::TimeRange;
use crate::discover::handler::TimeRangeBody;
use crate::discover::scope::{resolve as resolve_scope, Scope};
use crate::QueryState;

const DEFAULT_MAX_SOURCES: usize = 200;

/// RRF-style constant for rank→score when the graph provider returns no
/// per-hit score (today's `SearchHits` carries none). Higher-ranked hits get a
/// larger `1 / (GRAPH_RRF_K + rank)`; the value matches the data arm's `RRF_K`
/// so cross-mode scores stay on a comparable scale.
const GRAPH_RRF_K: f64 = 60.0;
/// Cap on how many databases the `graph: None` (all-graphs) fan-out enumerates,
/// mirroring the data arm's source bound so a broad graph search can't run an
/// unbounded number of per-graph SQL queries.
const GRAPH_MAX_DATABASES: usize = 200;

/// Handles `unified_search` needs to run any mode. Derived from what
/// `search_handler` + the memory `retrieve()` path require: the catalog + segment
/// format + node id drive the data fan-out; `pool` backs the memory/graph arms;
/// `tenant` + `principal` scope + RBAC-filter resolved sources.
#[derive(Clone)]
pub struct SearchCtx {
    pub catalog: Arc<dyn kyma_core::catalog::Catalog>,
    pub format: Arc<dyn kyma_core::segment_format::SegmentFormat>,
    pub node_id: Option<kyma_core::types::NodeId>,
    /// Catalog Postgres pool (`None` in local mode). Used by the memory/graph
    /// arms; the data arm does not need it.
    pub pool: Option<Arc<sqlx::PgPool>>,
    pub tenant: kyma_core::tenant::TenantId,
    /// Allowed-database RBAC filter, if the principal is scoped. `None` means
    /// no restriction (all resolved sources pass).
    pub allowed_databases: Option<Vec<String>>,
}

impl SearchCtx {
    /// Build a `SearchCtx` from the query-surface state + the request's principal.
    pub fn from_query_state(
        state: &QueryState,
        principal: Option<&crate::auth::Principal>,
    ) -> Self {
        let tenant = principal
            .map(|p| p.tenant)
            .unwrap_or(kyma_core::tenant::DEFAULT_TENANT);
        let allowed_databases = principal.and_then(|p| p.allowed_databases.clone());
        SearchCtx {
            catalog: state.catalog.clone(),
            format: state.format.clone(),
            node_id: state.node_id,
            pool: state.pg_pool.clone(),
            tenant,
            allowed_databases,
        }
    }
}

/// A unified search request. `query`/`scope`/`limit`/`time_range` drive the data
/// arm; the remaining fields are memory/graph passthrough. Every field is
/// `#[serde(default)]` so the legacy `{ "query", "scope", ... }` body parses
/// unchanged.
#[derive(Debug, Default, Deserialize)]
pub struct UnifiedSearchRequest {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub scope: Option<Scope>,
    #[serde(default)]
    pub time_range: Option<TimeRangeBody>,

    // ── memory/graph passthrough ──
    #[serde(default)]
    pub realms: Option<Vec<String>>,
    #[serde(default)]
    pub memory_type: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub importance_min: Option<f64>,
    #[serde(default)]
    pub as_of: Option<String>,
    #[serde(default)]
    pub include_invalidated: Option<bool>,
    #[serde(default)]
    pub expand_hops: Option<usize>,
    #[serde(default)]
    pub graph: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
}

/// Map data fan-out tuples → unified hits.
///
/// `kind` is `None` (omitted in JSON) so the data-mode envelope is byte-identical
/// to the legacy `SearchResponse` — see the module-level backward-compat note.
fn map_data_hits(rows: Vec<(String, f64, Value)>) -> Vec<UnifiedHit> {
    rows.into_iter()
        .map(|(source, score, row)| UnifiedHit {
            score,
            source,
            kind: None,
            id: None,
            title: None,
            row: Some(row),
            content_preview: None,
            memory_type: None,
        })
        .collect()
}

/// Build the data-mode envelope: legacy-shaped (no `mode`/`context`/`linked`).
fn data_response(
    rows: Vec<(String, f64, Value)>,
    sources_searched: usize,
    elapsed_ms: u64,
) -> UnifiedSearchResponse {
    UnifiedSearchResponse {
        hits: map_data_hits(rows),
        sources_searched,
        elapsed_ms,
        mode: None,
        context: None,
        linked: None,
    }
}

/// Build the memory-arm `RetrieveRequest` from the unified request's memory
/// passthrough fields. Maps 1:1 with the existing `recall_memory` /
/// `memory_search` tools, narrowing the wider unified types (`usize`/`f64`) to
/// the retrieval types (`u8`/`f32`); `retrieve()` itself clamps + defaults
/// `limit`/`expand_hops` from per-tenant settings when these are `None`.
fn memory_request(req: &UnifiedSearchRequest) -> RetrieveRequest {
    RetrieveRequest {
        query: req.query.clone(),
        realms: req.realms.clone().unwrap_or_default(),
        memory_type: req.memory_type.clone(),
        tags: req.tags.clone().unwrap_or_default(),
        importance_min: req.importance_min.map(|v| v as f32),
        as_of: req.as_of.clone(),
        include_invalidated: req.include_invalidated.unwrap_or(false),
        limit: req.limit,
        expand_hops: req.expand_hops.map(|h| h as u8),
    }
}

/// Map a memory `RetrieveResult` → the unified envelope, preserving recall's
/// ranked order and its full richness (graph-expanded `linked` resources + the
/// ready-to-use `context` block). Each memory becomes a `UnifiedHit` with
/// `kind:"memory"`, `source` set to its realm, and the row slot left empty.
///
/// `sources_searched` is the number of memories returned: memory recall fuses
/// across the whole `memory` graph rather than a fixed source set, so the
/// hit count is the meaningful "how much did we surface" signal (the realm
/// filter, if any, is carried per-hit in `source`).
fn map_memory_result(result: RetrieveResult) -> UnifiedSearchResponse {
    let hits = result
        .memories
        .into_iter()
        .map(|m| UnifiedHit {
            score: m.score,
            source: m.realm,
            kind: Some("memory".to_string()),
            id: Some(m.id),
            title: m.title,
            row: None,
            content_preview: Some(m.content_preview),
            memory_type: Some(m.memory_type),
        })
        .collect::<Vec<_>>();

    let context = (!result.context.is_empty()).then_some(result.context);
    let linked = (!result.linked.is_empty()).then(|| {
        result
            .linked
            .iter()
            .map(|l| serde_json::to_value(l).unwrap_or(Value::Null))
            .collect::<Vec<Value>>()
    });

    UnifiedSearchResponse {
        sources_searched: hits.len(),
        hits,
        elapsed_ms: result.took_ms as u64,
        mode: Some(mode_str(SearchMode::Memory).to_string()),
        context,
        linked,
    }
}

/// An empty, mode-echoed envelope (no hits, no fabricated data — just the mode
/// tag). All three arms are implemented now; this remains as a test helper for
/// asserting the empty-but-mode-tagged shape every arm degrades to.
#[cfg(test)]
fn empty_response(mode: SearchMode, elapsed_ms: u64) -> UnifiedSearchResponse {
    UnifiedSearchResponse {
        hits: Vec::new(),
        sources_searched: 0,
        elapsed_ms,
        mode: Some(mode_str(mode).to_string()),
        context: None,
        linked: None,
    }
}

fn mode_str(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Data => "data",
        SearchMode::Memory => "memory",
        SearchMode::Graph => "graph",
    }
}

/// A node's human title: its `properties.name` (if a string) else its id. The
/// stored-graph provider promotes/decodes `name` into `properties` when the
/// underlying table carries one (see `kyma_graph::stored_graph::collect_props`);
/// when it doesn't, the id is the only stable label we have.
fn node_title(node: &kyma_graph::GraphNode) -> String {
    node.properties
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| node.id.clone())
}

/// Map one graph's `SearchHits` → `UnifiedHit`s, tagging each with its
/// `"<db>/<graph>"` provenance and a rank-based score (the wire `SearchHits`
/// carries no per-hit score). `rank_offset` lets a multi-graph fan-out keep
/// later graphs' ranks from colliding with the first graph's top hits when the
/// merged list is globally re-sorted.
fn map_graph_hits(
    hits: kyma_graph::SearchHits,
    source: &str,
    rank_offset: usize,
) -> Vec<UnifiedHit> {
    hits.hits
        .into_iter()
        .enumerate()
        .map(|(rank, node)| {
            let title = node_title(&node);
            UnifiedHit {
                score: 1.0 / (GRAPH_RRF_K + (rank_offset + rank) as f64),
                source: source.to_string(),
                kind: Some("node".to_string()),
                id: Some(node.id),
                title: Some(title),
                row: None,
                content_preview: None,
                memory_type: None,
            }
        })
        .collect()
}

/// Shared in-process search substrate behind `POST /v1/search`.
///
/// Dispatches on `req.mode`. Errors are surfaced as an axum `Response` so the
/// HTTP handler can return them directly (matching the crate's
/// `error_response` style); successful results are a `UnifiedSearchResponse` the
/// handler serializes.
pub async fn unified_search(
    ctx: &SearchCtx,
    req: UnifiedSearchRequest,
    request_id: &str,
) -> Result<UnifiedSearchResponse, axum::response::Response> {
    let start = Instant::now();
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    match req.mode {
        SearchMode::Data => {
            let time_range = parse_time_range(req.time_range.as_ref(), request_id)?;
            let scope = req.scope.clone().unwrap_or(Scope::All);
            let max_sources = std::env::var("KYMA_DISCOVER_MAX_SOURCES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_MAX_SOURCES);

            let mut sources =
                match resolve_scope(&scope, ctx.tenant, ctx.catalog.clone(), None, max_sources)
                    .await
                {
                    Ok(s) => s,
                    Err(e) => {
                        return Err(crate::error_response(
                            axum::http::StatusCode::BAD_REQUEST,
                            "scope_error",
                            &format!("{e}"),
                            request_id,
                        ))
                    }
                };
            if let Some(allowed) = &ctx.allowed_databases {
                sources.retain(|s| allowed.iter().any(|a| a == &s.db));
            }

            let qvec = embed_query(&req.query).await;
            let sources_searched = sources.len();
            let rows = search_data(
                sources,
                &req.query,
                qvec,
                time_range,
                limit,
                ctx.catalog.clone(),
                ctx.format.clone(),
                ctx.node_id,
            )
            .await;

            Ok(data_response(
                rows,
                sources_searched,
                start.elapsed().as_millis() as u64,
            ))
        }
        // Memory arm: delegate to the agent's graph-aware hybrid `retrieve()`
        // so unified search and the `recall_memory` / `memory_search` tools
        // share one retrieval path — full recall quality (RRF + semantic +
        // keyword + graph-expansion + bi-temporal validity) is preserved.
        SearchMode::Memory => {
            let shared = SharedToolCtx {
                catalog: ctx.catalog.clone(),
                format: ctx.format.clone(),
                pool: ctx.pool.as_deref().cloned(),
                memory: None,
            };
            let req = memory_request(&req);
            let result = retrieve(&shared, &req).await;
            Ok(map_memory_result(result))
        }
        // Graph arm: resolve the requested graph(s) through the same
        // `graph_handler::resolve_with` + `StoredGraphProvider` path that backs
        // `POST /v1/graph/<name>/search`, then map the provider's `SearchHits`
        // into the unified envelope. No fabricated data — an unknown graph or a
        // provider error degrades to an empty (mode-echoed) result.
        SearchMode::Graph => {
            let (hits, graphs_searched) = graph_search(ctx, &req, limit).await;
            Ok(UnifiedSearchResponse {
                sources_searched: graphs_searched,
                hits,
                elapsed_ms: start.elapsed().as_millis() as u64,
                mode: Some(mode_str(SearchMode::Graph).to_string()),
                context: None,
                linked: None,
            })
        }
    }
}

/// Graph-arm core: search one named graph (`req.graph = Some`) or fan out across
/// every stored graph in scope (`req.graph = None`), returning the merged hits
/// (globally re-ranked, capped to `limit`) plus the number of graphs searched.
///
/// Resolution reuses [`crate::graph_handler::resolve_with`]; the provider's
/// `search(text, labels, realm=None, limit, offset=0)` is the same call the HTTP
/// search handler makes. Any resolution/search failure for a given graph is
/// skipped (it contributes no hits) rather than failing the whole request, so a
/// single bad graph never turns into a 500.
async fn graph_search(
    ctx: &SearchCtx,
    req: &UnifiedSearchRequest,
    limit: usize,
) -> (Vec<UnifiedHit>, usize) {
    let labels = req.labels.clone().unwrap_or_default();

    // The set of `(database, graph_name)` pairs to search.
    let targets: Vec<(String, String)> = match &req.graph {
        Some(name) => match request_database(ctx, req) {
            // Explicit database (via `scope: Sources["db.*"]`): that one graph.
            Some(db) => vec![(db, name.clone())],
            // No explicit database: resolve the named graph across every
            // database in scope. Kyma's namespaces are composite (db/graph), so
            // a bare graph name can exist in more than one database — searching
            // all matches keeps the unified view db-agnostic, matching the
            // all-DB discovery the data arm does.
            None => enumerate_graphs(ctx)
                .await
                .into_iter()
                .filter(|(_, g)| g == name)
                .collect(),
        },
        None => enumerate_graphs(ctx).await,
    };

    if targets.is_empty() {
        return (Vec::new(), 0);
    }

    // Per-graph cap so one graph in a broad fan-out can't crowd out the rest;
    // the merged list is globally re-ranked and truncated to `limit` below.
    let ngraphs = targets.len();
    let per_graph = limit.div_ceil(ngraphs).max(1);

    let mut all: Vec<UnifiedHit> = Vec::new();
    let mut searched = 0usize;
    for (db, graph) in targets {
        let allowed = ctx.allowed_databases.clone();
        let provider = match crate::graph_handler::resolve_with(
            &ctx.catalog,
            &ctx.format,
            ctx.tenant,
            &graph,
            &db,
            allowed,
        )
        .await
        {
            Ok(p) => p,
            // Unknown graph / catalog error: skip, never 500.
            Err(_) => continue,
        };
        searched += 1;
        match provider
            .search(&req.query, &labels, None, per_graph, 0)
            .await
        {
            Ok(hits) => {
                let source = format!("{db}/{graph}");
                all.extend(map_graph_hits(hits, &source, 0));
            }
            // Provider/SQL error for this graph: skip its hits.
            Err(_) => continue,
        }
    }

    // Global re-rank (descending score) + truncate, mirroring the data arm.
    all.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    all.truncate(limit);
    (all, searched)
}

/// The explicit database a single-graph request targets, read from a
/// `scope: Sources` pattern (`"db.*"` / `"db"`). Returns `None` when the request
/// names no database (the caller then resolves the named graph across every
/// database in scope). An explicit database outside a scoped token's allow-list
/// is rejected (returns `None` → no targets → empty result, never a leak).
fn request_database(ctx: &SearchCtx, req: &UnifiedSearchRequest) -> Option<String> {
    let Some(Scope::Sources { sources }) = &req.scope else {
        return None;
    };
    let db = sources.iter().find_map(|s| db_of_pattern(s))?;
    // Respect the allow-list if the token is scoped.
    let allowed = ctx
        .allowed_databases
        .as_ref()
        .map(|a| a.iter().any(|d| d == &db))
        .unwrap_or(true);
    allowed.then_some(db)
}

/// Extract the database from a `"db.table"` / `"db.*"` / `"db"` source pattern.
fn db_of_pattern(pattern: &str) -> Option<String> {
    let db = pattern.split('.').next().unwrap_or("").trim();
    if db.is_empty() || db == "*" {
        None
    } else {
        Some(db.to_string())
    }
}

/// Enumerate every stored graph across the databases in scope as
/// `(database, graph_name)` pairs, applying the principal's `allowed_databases`
/// filter. Mirrors the all-DB discovery the data arm does over sources.
async fn enumerate_graphs(ctx: &SearchCtx) -> Vec<(String, String)> {
    let dbs = match ctx.catalog.list_databases_in_tenant(ctx.tenant).await {
        Ok(dbs) => dbs,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for db in dbs.into_iter().take(GRAPH_MAX_DATABASES) {
        if let Some(allowed) = &ctx.allowed_databases {
            if !allowed.iter().any(|a| a == &db) {
                continue;
            }
        }
        let regs = match ctx.catalog.list_graphs_in_tenant(ctx.tenant, &db).await {
            Ok(r) => r,
            Err(_) => continue, // per-DB error: skip silently
        };
        for r in regs {
            out.push((db.clone(), r.name));
        }
    }
    out
}

/// Parse an optional `TimeRangeBody` into a `TimeRange`, mapping failures to a
/// `bad_time_range` HTTP error (mirrors the legacy handler).
fn parse_time_range(
    tr: Option<&TimeRangeBody>,
    request_id: &str,
) -> Result<Option<TimeRange>, axum::response::Response> {
    match tr {
        None => Ok(None),
        Some(tr) => match crate::discover::handler::parse_time_range(tr) {
            Ok(t) => Ok(Some(t)),
            Err(msg) => Err(crate::error_response(
                axum::http::StatusCode::BAD_REQUEST,
                "bad_time_range",
                &msg,
                request_id,
            )),
        },
    }
}

/// Embed the query once via the process-shared model. Lexical-only (`None`) if
/// the query is empty or no embedder is available.
async fn embed_query(query: &str) -> Option<Vec<f32>> {
    if query.trim().is_empty() {
        return None;
    }
    match kyma_memory::shared_embedding().await {
        Ok(embedder) => embedder
            .embed(std::slice::from_ref(&query.to_string()))
            .await
            .ok()
            .and_then(|mut v| v.drain(..).next()),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn data_hits_map_to_row_kind_none() {
        let rows = vec![
            ("db.tbl".to_string(), 0.9, json!({ "msg": "hello" })),
            ("db.other".to_string(), 0.5, json!({ "msg": "world" })),
        ];
        let hits = map_data_hits(rows);
        assert_eq!(hits.len(), 2);
        let h = &hits[0];
        assert_eq!(h.source, "db.tbl");
        assert_eq!(h.score, 0.9);
        assert_eq!(h.row, Some(json!({ "msg": "hello" })));
        // Backward-compat: data hits omit `kind` so JSON == legacy SearchHit.
        assert_eq!(h.kind, None);
        assert_eq!(h.id, None);
        assert_eq!(h.title, None);
        assert_eq!(h.content_preview, None);
        assert_eq!(h.memory_type, None);
    }

    #[test]
    fn data_response_is_legacy_byte_compatible() {
        let rows = vec![("db.tbl".to_string(), 0.9, json!({ "msg": "hello" }))];
        let resp = data_response(rows, 3, 12);
        let got: Value = serde_json::to_value(&resp).unwrap();
        // Exact legacy shape: hits[].{source,score,row}, sources_searched, elapsed_ms.
        let want = json!({
            "hits": [{ "source": "db.tbl", "score": 0.9, "row": { "msg": "hello" } }],
            "sources_searched": 3,
            "elapsed_ms": 12
        });
        assert_eq!(got, want, "data-mode response must equal legacy shape");

        // And explicitly: no mode/context/linked/kind keys leak in.
        let s = serde_json::to_string(&resp).unwrap();
        for forbidden in ["\"mode\"", "\"context\"", "\"linked\"", "\"kind\""] {
            assert!(!s.contains(forbidden), "leaked {forbidden} in {s}");
        }
    }

    #[test]
    fn memory_and_graph_arms_echo_mode_and_stay_empty() {
        let m = empty_response(SearchMode::Memory, 1);
        assert!(m.hits.is_empty());
        assert_eq!(m.mode.as_deref(), Some("memory"));
        let g = empty_response(SearchMode::Graph, 1);
        assert!(g.hits.is_empty());
        assert_eq!(g.mode.as_deref(), Some("graph"));
    }

    #[test]
    fn legacy_body_parses_into_unified_request() {
        // The old SearchBody JSON must still parse and default to Data mode.
        let body = r#"{ "query": "x", "scope": { "kind": "all" }, "limit": 10 }"#;
        let req: UnifiedSearchRequest = serde_json::from_str(body).unwrap();
        assert_eq!(req.mode, SearchMode::Data);
        assert_eq!(req.query, "x");
        assert_eq!(req.limit, Some(10));
        assert!(req.scope.is_some());
    }

    // ── memory arm: mapping unit test ──────────────────────────────────────
    //
    // A full end-to-end parity harness (seed via MemoryWriter → call both
    // `retrieve()` and `unified_search(Memory)` and compare ids/order) is
    // infeasible in a unit/CI context: `retrieve()` builds its writer through
    // `kyma_memory::shared_embedding()`, a process-wide OnceCell that downloads
    // and loads a real fastembed ONNX model (~30–130 MB, network) with no
    // test-time injection seam — and that 384-dim embedder mismatches any
    // MockEmbed-seeded vectors. So we test the **mapping** (the load-bearing
    // logic this arm introduces) directly against a synthetic RetrieveResult,
    // plus a smoke test that the arm runs over a real (empty) catalog and
    // degrades like `retrieve()` does. The mapping function is the single point
    // where `RetrieveResult` becomes `UnifiedSearchResponse`, so asserting it
    // exhaustively is what guarantees rich-recall parity.

    use crate::agent::memory_retrieve::{LinkedResource, RetrieveResult, RetrievedMemory};

    fn sample_memory(id: &str, score: f64, realm: &str) -> RetrievedMemory {
        RetrievedMemory {
            id: id.to_string(),
            memory_type: "decision".to_string(),
            title: Some(format!("title-{id}")),
            content_preview: format!("preview of {id}"),
            score,
            distance: Some(0.1),
            kw_score: Some(0.2),
            graph_proximity: 0.5,
            importance: 0.7,
            realm: realm.to_string(),
            valid_at: Some("2026-01-01T00:00:00Z".to_string()),
            invalid_at: None,
            via: None,
        }
    }

    #[test]
    fn memory_result_maps_to_unified_response() {
        let result = RetrieveResult {
            memories: vec![
                sample_memory("memory:a", 0.9, "proj"),
                sample_memory("memory:b", 0.4, "global"),
            ],
            linked: vec![LinkedResource {
                node_id: "repo:acme/app".to_string(),
                target_namespace: Some("github".to_string()),
                edge_type: "REFERENCES".to_string(),
                depth: 1,
            }],
            context: "Relevant memories:\n- ...".to_string(),
            took_ms: 42,
        };

        let resp = map_memory_result(result.clone());

        // hits preserve order + carry every memory-shaped field.
        assert_eq!(resp.hits.len(), 2);
        let h0 = &resp.hits[0];
        assert_eq!(h0.kind.as_deref(), Some("memory"));
        assert_eq!(h0.id.as_deref(), Some("memory:a"));
        assert_eq!(h0.title.as_deref(), Some("title-memory:a"));
        assert_eq!(h0.content_preview.as_deref(), Some("preview of memory:a"));
        assert_eq!(h0.memory_type.as_deref(), Some("decision"));
        assert_eq!(h0.source, "proj"); // source = realm for memory hits
        assert_eq!(h0.score, 0.9);
        assert_eq!(h0.row, None); // memory hits never carry a row
        // order matches retrieve()'s ranked order.
        assert_eq!(resp.hits[1].id.as_deref(), Some("memory:b"));

        // context echoed through verbatim.
        assert_eq!(resp.context.as_deref(), Some("Relevant memories:\n- ..."));

        // linked carries the resources, each serialized as the LinkedResource JSON.
        let linked = resp.linked.as_ref().expect("linked present");
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0]["node_id"], "repo:acme/app");
        assert_eq!(linked[0]["target_namespace"], "github");
        assert_eq!(linked[0]["edge_type"], "REFERENCES");
        assert_eq!(linked[0]["depth"], 1);

        // envelope metadata: mode tag, elapsed from took_ms, sources = hit count.
        assert_eq!(resp.mode.as_deref(), Some("memory"));
        assert_eq!(resp.elapsed_ms, 42);
        assert_eq!(resp.sources_searched, 2);
    }

    #[test]
    fn memory_result_empty_omits_context_and_linked() {
        let resp = map_memory_result(RetrieveResult::default());
        assert!(resp.hits.is_empty());
        assert_eq!(resp.mode.as_deref(), Some("memory"));
        // No memories ⇒ build_context returns "" ⇒ context omitted; no linked.
        assert_eq!(resp.context, None);
        assert_eq!(resp.linked, None);
        assert_eq!(resp.sources_searched, 0);
    }

    // ── memory arm: smoke test over a real (empty) catalog ─────────────────

    async fn empty_ctx() -> SearchCtx {
        use kyma_core::segment_format::SegmentFormat;
        let catalog: Arc<dyn kyma_core::catalog::Catalog> = Arc::new(
            kyma_catalog_sqlite::SqliteCatalog::connect_in_memory()
                .await
                .expect("in-memory catalog"),
        );
        let tmp = std::env::temp_dir().join(format!("kyma-usearch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = kyma_storage::build_object_store(&kyma_storage::StorageConfig::Local {
            root: tmp.to_string_lossy().to_string(),
        })
        .unwrap();
        let format: Arc<dyn SegmentFormat> = Arc::new(kyma_format_tlm::TelemetryFormat::new(store, "test"));
        SearchCtx {
            catalog,
            format,
            node_id: None,
            pool: None,
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            allowed_databases: None,
        }
    }

    #[tokio::test]
    async fn memory_arm_runs_and_degrades_gracefully_on_empty_store() {
        // No memory store provisioned + no embedder configured for the env:
        // the arm must still run through `retrieve()` and return a well-formed,
        // mode-tagged, empty envelope (never panic), exactly like `retrieve()`
        // degrades to zero results.
        let ctx = empty_ctx().await;
        let req = UnifiedSearchRequest {
            query: "anything".to_string(),
            mode: SearchMode::Memory,
            realms: Some(vec!["proj".to_string()]),
            expand_hops: Some(1),
            ..Default::default()
        };
        let resp = unified_search(&ctx, req, "req-mem-smoke")
            .await
            .expect("memory arm returns Ok");
        assert_eq!(resp.mode.as_deref(), Some("memory"));
        assert!(resp.hits.is_empty(), "empty store ⇒ no hits");
        // Every hit (if any were present) would carry kind:"memory".
        assert!(resp.hits.iter().all(|h| h.kind.as_deref() == Some("memory")));
    }

    // ── graph arm: mapping unit test ───────────────────────────────────────

    fn graph_node(id: &str, label: &str, name: Option<&str>) -> kyma_graph::GraphNode {
        let mut properties = kyma_graph::types::Props::new();
        if let Some(n) = name {
            properties.insert("name".into(), serde_json::json!(n));
        }
        kyma_graph::GraphNode {
            id: id.to_string(),
            labels: vec![label.to_string()],
            properties,
            metadata: kyma_graph::types::NodeMetadata {
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                source_type: Some("stored".into()),
                source_id: None,
                realm: "kg".into(),
            },
        }
    }

    #[test]
    fn graph_hits_map_to_node_kind_with_source_and_title() {
        let hits = kyma_graph::SearchHits {
            hits: vec![
                graph_node("svc:alpha", "Service", Some("alpha-service")),
                graph_node("svc:beta", "Service", None), // no name ⇒ title falls back to id
            ],
            total: 2,
            limit: 20,
            offset: 0,
        };
        let mapped = map_graph_hits(hits, "kg/kg", 0);
        assert_eq!(mapped.len(), 2);

        let h0 = &mapped[0];
        assert_eq!(h0.kind.as_deref(), Some("node"));
        assert_eq!(h0.id.as_deref(), Some("svc:alpha"));
        assert_eq!(h0.title.as_deref(), Some("alpha-service")); // properties.name
        assert_eq!(h0.source, "kg/kg");
        assert_eq!(h0.row, None);
        assert_eq!(h0.content_preview, None);

        // No `name` ⇒ title is the node id.
        assert_eq!(mapped[1].title.as_deref(), Some("svc:beta"));

        // Rank-based scores are strictly descending (first hit ranks highest).
        assert!(mapped[0].score > mapped[1].score);

        // `rank_offset` shifts the rank denominator so a later graph's hits
        // can't tie the first graph's top hit.
        let offset = map_graph_hits(
            kyma_graph::SearchHits {
                hits: vec![graph_node("svc:gamma", "Service", None)],
                total: 1,
                limit: 20,
                offset: 0,
            },
            "kg/kg",
            5,
        );
        assert!(offset[0].score < mapped[0].score);
    }

    // ── graph arm: full end-to-end over a seeded stored graph ──────────────
    //
    // Unlike the memory arm (whose `retrieve()` needs a real ONNX embedder),
    // the graph arm runs pure SQL through the same `StoredGraphProvider` +
    // DataFusion executor the HTTP `/v1/graph/<g>/search` uses. That path runs
    // fully in-process over an in-memory SQLite catalog + local object store, so
    // we seed a real stored graph end-to-end: create the node table, ingest node
    // rows (one whose name contains the search term), register the graph, then
    // assert `unified_search(mode:Graph)` surfaces it as a `node` hit.

    /// Build a `SearchCtx` + ingest `kg.kg_nodes` with the given NDJSON, then
    /// register a `"kg"` stored graph over it. Returns the ctx.
    async fn ctx_with_seeded_graph(node_ndjson: &str) -> SearchCtx {
        use kyma_core::catalog::{GraphSpec, TableConfig};
        use kyma_core::segment_format::SegmentFormat;
        use std::sync::Arc;

        let catalog: Arc<dyn kyma_core::catalog::Catalog> = Arc::new(
            kyma_catalog_sqlite::SqliteCatalog::connect_in_memory()
                .await
                .expect("in-memory catalog"),
        );
        let tmp = std::env::temp_dir().join(format!("kyma-graph-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = kyma_storage::build_object_store(&kyma_storage::StorageConfig::Local {
            root: tmp.to_string_lossy().to_string(),
        })
        .unwrap();
        let format: Arc<dyn SegmentFormat> =
            Arc::new(kyma_format_tlm::TelemetryFormat::new(store, "test"));

        // db "kg" + node table kg_nodes(id, labels, name, realm).
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
        // An edge table so the registration is complete (search never touches it).
        catalog
            .create_table(db_id, "kg_edges", schema.clone(), TableConfig::default())
            .await
            .expect("create kg_edges");

        // Ingest node rows via the real write path.
        let tref = catalog.lookup_table("kg", "kg_nodes").await.expect("lookup kg_nodes");
        let batches = kyma_ingest_core::parse_ndjson(node_ndjson.as_bytes(), tref.schema.clone())
            .expect("parse ndjson");
        kyma_ingest_core::WritePath::new(catalog.clone(), format.clone())
            .ingest("kg", &tref, batches)
            .await
            .expect("ingest node rows");

        // Register the "kg" stored graph (id/labels roles; realm optional).
        let mut spec = GraphSpec::with_defaults("kg_nodes", "kg_edges");
        spec.realm_col = Some("realm".into());
        catalog
            .create_graph("kg", "kg", spec)
            .await
            .expect("create_graph kg");

        SearchCtx {
            catalog,
            format,
            node_id: None,
            pool: None,
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            allowed_databases: None,
        }
    }

    #[tokio::test]
    async fn graph_arm_returns_node_hits_from_seeded_stored_graph() {
        let ctx = ctx_with_seeded_graph(
            r#"{"id":"svc:alpha","labels":"Service","name":"alpha-service","realm":"kg"}
{"id":"svc:beta","labels":"Service","name":"beta-service","realm":"kg"}"#,
        )
        .await;

        let req = UnifiedSearchRequest {
            query: "alpha".to_string(),
            mode: SearchMode::Graph,
            graph: Some("kg".to_string()),
            scope: Some(Scope::Sources {
                sources: vec!["kg.*".to_string()],
            }),
            ..Default::default()
        };
        let resp = unified_search(&ctx, req, "req-graph-e2e")
            .await
            .expect("graph arm returns Ok");

        assert_eq!(resp.mode.as_deref(), Some("graph"));
        assert_eq!(resp.sources_searched, 1, "one graph searched");
        assert!(!resp.hits.is_empty(), "expected a hit for 'alpha'");

        let hit = resp
            .hits
            .iter()
            .find(|h| h.id.as_deref() == Some("svc:alpha"))
            .expect("svc:alpha must be a hit");
        assert_eq!(hit.kind.as_deref(), Some("node"));
        assert_eq!(hit.title.as_deref(), Some("alpha-service"));
        assert_eq!(hit.source, "kg/kg", "source is <db>/<graph>");
        // The non-matching node must not surface for the 'alpha' query.
        assert!(resp.hits.iter().all(|h| h.id.as_deref() != Some("svc:beta")));
    }

    #[tokio::test]
    async fn graph_arm_resolves_named_graph_across_dbs_without_explicit_scope() {
        // Same seed, but the request gives no `scope` → the arm resolves the
        // named graph across every database in scope (here just "kg").
        let ctx = ctx_with_seeded_graph(
            r#"{"id":"svc:alpha","labels":"Service","name":"alpha-service","realm":"kg"}"#,
        )
        .await;
        let req = UnifiedSearchRequest {
            query: "alpha".to_string(),
            mode: SearchMode::Graph,
            graph: Some("kg".to_string()),
            ..Default::default()
        };
        let resp = unified_search(&ctx, req, "req-graph-nodb")
            .await
            .expect("graph arm returns Ok");
        assert_eq!(resp.sources_searched, 1);
        assert!(resp.hits.iter().any(|h| h.id.as_deref() == Some("svc:alpha")));
        assert!(resp.hits.iter().all(|h| h.source == "kg/kg"));
    }

    #[tokio::test]
    async fn graph_arm_unknown_graph_returns_empty_never_500() {
        // No graph registered → resolution 404s internally → skipped → empty.
        let ctx = empty_ctx().await;
        let req = UnifiedSearchRequest {
            query: "anything".to_string(),
            mode: SearchMode::Graph,
            graph: Some("does-not-exist".to_string()),
            scope: Some(Scope::Sources {
                sources: vec!["nope.*".to_string()],
            }),
            ..Default::default()
        };
        let resp = unified_search(&ctx, req, "req-graph-unknown")
            .await
            .expect("graph arm never errors to 500");
        assert_eq!(resp.mode.as_deref(), Some("graph"));
        assert!(resp.hits.is_empty());
        assert_eq!(resp.sources_searched, 0);
    }

    #[tokio::test]
    async fn graph_arm_empty_catalog_no_graph_name_is_empty() {
        // `graph: None` over a catalog with no graphs ⇒ nothing to search.
        let ctx = empty_ctx().await;
        let req = UnifiedSearchRequest {
            query: "x".to_string(),
            mode: SearchMode::Graph,
            ..Default::default()
        };
        let resp = unified_search(&ctx, req, "req-graph-none")
            .await
            .expect("graph arm ok");
        assert!(resp.hits.is_empty());
        assert_eq!(resp.sources_searched, 0);
    }
}
