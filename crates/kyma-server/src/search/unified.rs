//! Unified search substrate: one in-process dispatcher behind `POST /v1/search`.
//!
//! `unified_search` is the shared entry point the HTTP handler routes through.
//! It selects a backend by [`SearchMode`] and returns a single
//! [`UnifiedSearchResponse`] envelope across data/memory/graph modes.
//!
//! Only the **Data** arm is implemented here; it reuses the existing per-source
//! lexical+vector RRF fan-out via [`super::search_data`] so the legacy data path
//! and the unified path share one code path. The `Memory` and `Graph` arms are
//! wired as empty-but-mode-echoed responses pending the next tasks — they return
//! no fabricated data.
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

use super::types::{SearchMode, UnifiedHit, UnifiedSearchResponse};
use super::{search_data, DEFAULT_LIMIT, MAX_LIMIT};
use crate::discover::compile::TimeRange;
use crate::discover::handler::TimeRangeBody;
use crate::discover::scope::{resolve as resolve_scope, Scope};
use crate::QueryState;

const DEFAULT_MAX_SOURCES: usize = 200;

/// Handles `unified_search` needs to run any mode. Derived from what
/// `search_handler` + the memory `retrieve()` path require: the catalog + segment
/// format + node id drive the data fan-out; `pool` backs the memory/graph arms;
/// `tenant` + `principal` scope + RBAC-filter resolved sources.
#[derive(Clone)]
pub struct SearchCtx {
    pub catalog: Arc<dyn kyma_core::catalog::Catalog>,
    pub format: Arc<dyn kyma_core::segment_format::SegmentFormat>,
    pub node_id: Option<kyma_core::types::NodeId>,
    /// Catalog Postgres pool (`None` in local mode). Threaded through for the
    /// memory/graph arms (next tasks); the data arm does not need it.
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
/// arm; the remaining fields are memory/graph passthrough for the next tasks.
/// Every field is `#[serde(default)]` so the legacy `{ "query", "scope", ... }`
/// body parses unchanged.
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

    // ── memory/graph passthrough (consumed by tasks 3/4) ──
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

/// An empty, mode-echoed envelope for not-yet-implemented arms. No fabricated
/// data — just the mode tag so callers can tell the arm ran.
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
        // TODO(piece-1 task 3): memory arm — route through `agent::memory_retrieve::retrieve`.
        SearchMode::Memory => Ok(empty_response(
            SearchMode::Memory,
            start.elapsed().as_millis() as u64,
        )),
        // TODO(piece-1 task 4): graph arm — route through the graph search path.
        SearchMode::Graph => Ok(empty_response(
            SearchMode::Graph,
            start.elapsed().as_millis() as u64,
        )),
    }
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
}
