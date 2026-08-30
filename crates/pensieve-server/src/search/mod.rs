//! Unified hybrid search — `POST /v1/search`.
//!
//! One instant, broad search used by the Explore UI *and* agents: for every
//! source in scope it runs a **lexical** leg (token-index `contains` over string
//! columns, via the Discover compiler) and, where the source has a vector
//! column, a **vector** leg (`cosine_distance` against the query embedding), then
//! fuses the two ranked lists with Reciprocal Rank Fusion. The query is embedded
//! at request time via the process-shared embedding backend
//! (`kyma_memory::shared_embedding`). Hits carry `db.table` provenance + the row,
//! so callers can follow up with SQL/KQL to correlate.
//!
//! Degrades gracefully: no embedder / no vector column → lexical-only; a failing
//! source is skipped rather than failing the whole request.

pub mod cache;
pub mod types;
pub mod unified;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use arrow::json::ArrayWriter;
use arrow_schema::DataType;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use datafusion::execution::memory_pool::GreedyMemoryPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::prelude::{SessionConfig, SessionContext};
use kyma_exec::KymaTable;
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::discover::compile::{compile_for_source, TimeRange};
use crate::discover::grammar::Clause;
use crate::discover::scope::ResolvedSource;
use crate::QueryState;

/// Reciprocal-rank-fusion constant in `1 / (RRF_K + rank)`.
const RRF_K: f64 = 60.0;
const PER_LEG_K: usize = 50;
/// Default/clamp bounds for `limit`, applied by `unified::unified_search`
/// (referenced from the child `unified` module).
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;
const MAX_BODY_BYTES: usize = 256 * 1024;

/// Per-source DataFusion memory pool. Each leg only materializes the top
/// `PER_LEG_K` rows, so a small bound is plenty — and it caps search's peak
/// memory so a burst of broad ("all sources") searches can't exhaust the
/// process. A leg that would exceed it errors and is skipped (lexical-only /
/// fewer hits) rather than crashing the server.
const PER_SOURCE_MEM_BUDGET: usize = 64 * 1024 * 1024;

/// Process-wide cap on concurrent per-source search legs. Each permit guards one
/// DataFusion context + memory pool, so peak search memory is bounded by
/// `permits × PER_SOURCE_MEM_BUDGET` regardless of how many requests arrive at
/// once or how broadly each fans out (a single "all" scope can resolve to
/// `unified::DEFAULT_MAX_SOURCES` sources). Without this, simultaneous broad searches
/// spin up unbounded contexts and can OOM/wedge the server — the legs still all
/// run, they just queue for a slot instead of all allocating at once.
fn source_search_limiter() -> &'static Semaphore {
    static LIMITER: OnceLock<Semaphore> = OnceLock::new();
    LIMITER.get_or_init(|| {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Semaphore::new(cores.clamp(2, 8))
    })
}

/// `POST /v1/search` — routes through the shared [`unified::unified_search`]
/// substrate. The request parses into a [`unified::UnifiedSearchRequest`]; the
/// legacy data body (`{ "query", "scope", "time_range?", "limit?" }`) is a
/// subset of it and parses unchanged (defaulting to `mode: "data"`). For
/// `mode == "data"` the serialized response is byte-compatible with the legacy
/// `{ "hits": [{ "source", "score", "row" }], "sources_searched", "elapsed_ms" }`
/// shape — see `unified.rs`'s backward-compat note.
pub async fn search_handler(State(state): State<QueryState>, req: Request<Body>) -> Response {
    let (parts, body) = req.into_parts();
    let request_id = crate::extract_request_id(&parts.headers);

    // Shared query admission control (see `crate::concurrency`); permit held for
    // the duration of the search. No-op unless KYMA_QUERY_MAX_CONCURRENT is set.
    let _admission = match crate::concurrency::acquire() {
        Ok(p) => p,
        Err(retry) => return crate::too_many_requests_response(retry, &request_id),
    };

    let body_bytes: Bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return crate::error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                &format!("failed to read body: {e}"),
                &request_id,
            )
        }
    };
    let payload: unified::UnifiedSearchRequest = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(e) => {
            return crate::error_response(
                StatusCode::BAD_REQUEST,
                "bad_request",
                &format!("invalid JSON body: {e}"),
                &request_id,
            )
        }
    };

    let principal = parts.extensions.get::<crate::auth::Principal>();
    let ctx = unified::SearchCtx::from_query_state(&state, principal);

    let resp = match unified::unified_search(&ctx, payload, &request_id).await {
        Ok(r) => r,
        Err(err_response) => return err_response,
    };

    let body = match serde_json::to_vec(&resp) {
        Ok(b) => b,
        Err(e) => {
            return crate::error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                &format!("{e}"),
                &request_id,
            )
        }
    };
    let mut out = Response::new(Body::from(body));
    let h = out.headers_mut();
    h.insert(
        "content-type",
        axum::http::HeaderValue::from_static("application/json"),
    );
    if let Ok(rid) = axum::http::HeaderValue::from_str(&request_id) {
        h.insert("x-request-id", rid);
    }
    out
}

/// Data-mode fan-out core, shared by the legacy `search_handler` and the
/// unified `unified_search` dispatcher.
///
/// Fans out per resolved source (each leg gated on the process-wide
/// `source_search_limiter` + bounded memory pool inside `search_one_source`),
/// collects all `(source_key, score, row)` tuples, ranks globally by fused RRF
/// score, and truncates to `limit`. A failing source is skipped (its join task
/// yields an empty vec) rather than failing the whole search.
pub(crate) async fn search_data(
    sources: Vec<ResolvedSource>,
    query: &str,
    qvec: Option<Vec<f32>>,
    time_range: Option<TimeRange>,
    limit: usize,
    catalog: Arc<dyn kyma_core::catalog::Catalog>,
    format: Arc<dyn kyma_core::segment_format::SegmentFormat>,
    node_id: Option<kyma_core::types::NodeId>,
    tenant: kyma_core::tenant::TenantId,
) -> Vec<(String, f64, Value)> {
    // Fan out per source.
    let mut set: JoinSet<Vec<(String, f64, Value)>> = JoinSet::new();
    for src in sources {
        let catalog = catalog.clone();
        let format = format.clone();
        let query = query.to_string();
        let qv = qvec.clone();
        let tr = time_range;
        set.spawn(async move {
            search_one_source(
                src,
                &query,
                qv.as_deref(),
                tr,
                catalog,
                format,
                node_id,
                tenant,
            )
            .await
        });
    }

    let mut hits: Vec<(String, f64, Value)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(rows) = joined {
            hits.extend(rows);
        }
    }

    // Global ranking by fused RRF score.
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // S1.6 cross-encoder rerank stage (no-op unless a reranker is configured):
    // re-score the top candidates jointly against the query and re-order. Off by
    // default (zero overhead), bounded to RERANK_CANDIDATES so latency stays
    // capped; a rerank failure falls back to RRF order.
    rerank_stage(query, hits, limit).await
}

/// Max candidates fed to the cross-encoder (latency cap). The reranker scores
/// the RRF-top of this many rows; the rest can't beat them on RRF anyway.
const RERANK_CANDIDATES: usize = 50;

/// Apply the cross-encoder reranker to the RRF-sorted `hits` when one is
/// configured; otherwise just truncate to `limit`. Pure re-ordering lives in
/// [`apply_rerank_scores`] for testability.
async fn rerank_stage(
    query: &str,
    mut hits: Vec<(String, f64, Value)>,
    limit: usize,
) -> Vec<(String, f64, Value)> {
    let Some(reranker) = kyma_memory::shared_reranker().await else {
        hits.truncate(limit);
        return hits;
    };
    let cand = hits.len().min(RERANK_CANDIDATES.max(limit));
    let head: Vec<(String, f64, Value)> = hits.into_iter().take(cand).collect();
    let docs: Vec<String> = head.iter().map(|(_, _, row)| row_text(row)).collect();
    match reranker.rerank(query, &docs).await {
        Ok(scores) if scores.len() == head.len() => apply_rerank_scores(head, scores, limit),
        _ => {
            // Rerank unavailable / shape mismatch → keep RRF order.
            let mut out = head;
            out.truncate(limit);
            out
        }
    }
}

/// Re-order `head` by cross-encoder `scores` (aligned by index), replace each
/// row's score with its rerank score, and keep the top `limit`. Pure — tested.
fn apply_rerank_scores(
    head: Vec<(String, f64, Value)>,
    scores: Vec<f32>,
    limit: usize,
) -> Vec<(String, f64, Value)> {
    let mut scored: Vec<(f32, (String, f64, Value))> = scores.into_iter().zip(head).collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(limit)
        .map(|(rs, (sk, _rrf, row))| (sk, rs as f64, row))
        .collect()
}

/// A doc text for the cross-encoder: the row's non-internal string fields joined,
/// bounded so a huge row can't blow up tokenization latency.
fn row_text(row: &Value) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Value::Object(map) = row {
        for (k, v) in map {
            if k.starts_with("__") {
                continue; // internal (__score, etc.)
            }
            if let Value::String(s) = v {
                if !s.is_empty() {
                    parts.push(s);
                }
            }
        }
    }
    let mut text = parts.join(" ");
    if text.len() > 2000 {
        // Truncate on a char boundary.
        let mut end = 2000;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    text
}

/// Run both legs for one source, RRF-fuse, return `(source_key, score, row)`.
#[allow(clippy::too_many_arguments)]
async fn search_one_source(
    src: ResolvedSource,
    query: &str,
    qvec: Option<&[f32]>,
    time_range: Option<TimeRange>,
    catalog: Arc<dyn kyma_core::catalog::Catalog>,
    format: Arc<dyn kyma_core::segment_format::SegmentFormat>,
    node_id: Option<kyma_core::types::NodeId>,
    tenant: kyma_core::tenant::TenantId,
) -> Vec<(String, f64, Value)> {
    let source_key = format!("{}.{}", src.db, src.table.name);
    let table_name = src.table.name.clone();

    // Gate the heavy work (context + memory pool + scans) on a process-wide
    // permit so concurrent/broad searches queue for a slot instead of all
    // allocating at once. Held until this leg returns. Err only if the
    // semaphore were closed (never), in which case we proceed unbounded.
    let _permit = source_search_limiter().acquire().await.ok();

    // Fresh, modestly-bounded context.
    let runtime = match RuntimeEnvBuilder::new()
        .with_memory_pool(Arc::new(GreedyMemoryPool::new(PER_SOURCE_MEM_BUDGET)))
        .build()
    {
        Ok(r) => Arc::new(r),
        Err(_) => return Vec::new(),
    };
    let ctx = SessionContext::new_with_config_rt(SessionConfig::new(), runtime);
    kyma_exec::register_vector_udfs(&ctx);
    let kt: Arc<KymaTable> = match node_id {
        Some(nid) => Arc::new(KymaTable::with_node_id(
            src.table.clone(),
            catalog.clone(),
            format.clone(),
            nid,
            src.db.clone(),
        )),
        None => Arc::new(KymaTable::new(
            src.table.clone(),
            catalog.clone(),
            format.clone(),
        )),
    };
    if ctx.register_table(&table_name, kt).is_err() {
        return Vec::new();
    }

    let vec_col = vector_column(&src.table);
    let non_vector_cols: Vec<String> = src
        .table
        .schema
        .fields()
        .iter()
        .filter(|f| !is_vector_type(f.data_type()))
        .map(|f| f.name().clone())
        .collect();

    // ── Lexical leg: BM25 over a tantivy FTS sidecar when present, else the
    //    token-index `contains`/LIKE compile. Capability-gated like the vector
    //    leg, so tables without FTS sidecars behave exactly as before.
    let lexical_rows = match bm25_lexical_rows(
        &catalog,
        &format,
        tenant,
        &src.table,
        &query,
        &non_vector_cols,
    )
    .await
    {
        Some(rows) => rows,
        None => {
            let clauses: Vec<Clause> = if query.trim().is_empty() {
                Vec::new()
            } else {
                vec![Clause::Substring {
                    value: query.to_string(),
                }]
            };
            let compiled = compile_for_source(&src.table, &clauses, time_range.as_ref(), PER_LEG_K);
            match kyma_kql::kql_to_sql(&compiled.kql) {
                Ok(sql) => run_rows(&ctx, &sql).await,
                Err(_) => Vec::new(),
            }
        }
    };

    // ── Vector leg (ANN sidecar when available, else SQL cosine_distance) ──
    //
    // Capability gate: only switch to the IVF+RaBitQ ANN path when ≥1 sidecar
    // exists for this table's vector column. Deployments/tests with no sidecars
    // fall through to the exact SQL `ORDER BY cosine_distance(...)` path,
    // unchanged — which keeps existing search behaviour and tests intact. Both
    // paths emit the SAME `(non_vector_cols…, __score)` row shape so downstream
    // RRF fusion is identical.
    let vector_rows = match (vec_col.as_deref(), qvec) {
        (Some(col), Some(qv)) if !non_vector_cols.is_empty() => {
            match ann_vector_rows(
                &catalog,
                &format,
                tenant,
                &src.table,
                col,
                qv,
                &non_vector_cols,
            )
            .await
            {
                Some(rows) => rows,
                None => {
                    let proj = non_vector_cols
                        .iter()
                        .map(|c| quote_ident(c))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let sql = format!(
                        "SELECT {proj}, cosine_distance({vc}, {arr}) AS __score FROM {tbl} ORDER BY __score ASC LIMIT {k}",
                        vc = quote_ident(col),
                        arr = make_array(qv),
                        tbl = quote_ident(&table_name),
                        k = PER_LEG_K,
                    );
                    run_rows(&ctx, &sql).await
                }
            }
        }
        _ => Vec::new(),
    };

    // ── RRF fusion (keyed by stable row content) ──
    // Strip the vector column as well as the score: a leg that still carries
    // the embedding (or any future leg asymmetry) must not defeat the
    // row-content dedup key or bloat the hit payload.
    let mut fused: BTreeMap<String, (f64, Value)> = BTreeMap::new();
    for (rank, mut row) in lexical_rows.into_iter().enumerate() {
        strip(&mut row, "__score");
        if let Some(vc) = &vec_col {
            strip(&mut row, vc);
        }
        let key = row_key(&row);
        let e = fused.entry(key).or_insert((0.0, row));
        e.0 += 1.0 / (RRF_K + rank as f64);
    }
    for (rank, mut row) in vector_rows.into_iter().enumerate() {
        strip(&mut row, "__score");
        if let Some(vc) = &vec_col {
            strip(&mut row, vc);
        }
        let key = row_key(&row);
        let e = fused.entry(key).or_insert((0.0, row));
        e.0 += 1.0 / (RRF_K + rank as f64);
    }

    fused
        .into_values()
        .map(|(score, row)| (source_key.clone(), score, row))
        .collect()
}

/// Execute SQL on `ctx` and collect rows as JSON objects (best-effort).
async fn run_rows(ctx: &SessionContext, sql: &str) -> Vec<Value> {
    let df = match ctx.sql(sql).await {
        Ok(df) => df,
        Err(_) => return Vec::new(),
    };
    let batches = match df.collect().await {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let mut bytes: Vec<u8> = Vec::new();
    for batch in &batches {
        let mut w = ArrayWriter::new(&mut bytes);
        if w.write(batch).is_err() || w.finish().is_err() {
            return Vec::new();
        }
    }
    let mut out: Vec<Value> = Vec::new();
    let stream = serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>();
    for arr in stream.flatten() {
        match arr {
            Value::Array(rows) => out.extend(rows),
            other => out.push(other),
        }
    }
    out
}

/// Cosine-distance vector leg over the per-extent IVF+RaBitQ ANN sidecars.
///
/// Returns `Some(rows)` — JSON rows shaped exactly like the SQL leg
/// (`non_vector_cols…` + `__score`, ascending distance) — when the table has at
/// least one ANN sidecar for `vec_col` AND the format exposes an object store.
/// Returns `None` to signal the caller to fall back to the exact SQL path
/// (no sidecar, no store, or a hard error — never a silent empty result that
/// would defeat fusion).
async fn ann_vector_rows(
    catalog: &Arc<dyn kyma_core::catalog::Catalog>,
    format: &Arc<dyn kyma_core::segment_format::SegmentFormat>,
    tenant: kyma_core::tenant::TenantId,
    table: &kyma_core::catalog::TableRef,
    vec_col: &str,
    qvec: &[f32],
    non_vector_cols: &[String],
) -> Option<Vec<Value>> {
    use kyma_core::index_sidecar::SidecarKind;

    // Capability gate: need an object store (object-store-backed format) AND at
    // least one IvfRabitq sidecar on this column. Either missing → SQL path.
    let store = format.object_store()?;

    let extents = catalog
        .list_extents_in_tenant(
            tenant,
            table.id,
            table.current_snapshot_id,
            &kyma_core::catalog::PrunePredicate::default(),
        )
        .await
        .ok()?;
    if extents.is_empty() {
        return None;
    }
    let extent_ids: Vec<_> = extents.iter().map(|m| m.id).collect();
    let sidecars = catalog
        .list_index_sidecars(tenant, table.id, &extent_ids, Some(SidecarKind::IvfRabitq))
        .await
        .ok()?;
    if !sidecars.iter().any(|d| d.column == vec_col) {
        return None; // no ANN index for this column → SQL fallback
    }

    let cache = sidecar_cache();
    let params = kyma_exec::AnnParams::with_k(PER_LEG_K);
    let hits = match kyma_exec::ann_topk(
        catalog, tenant, format, &store, cache, table, vec_col, qvec, &params, None,
    )
    .await
    {
        Ok(h) => h,
        // Hard error in the ANN path → fall back to SQL rather than drop the leg.
        Err(_) => return None,
    };

    // Resolve each hit (extent, block, row) back to its non-vector column values
    // so the row shape matches the SQL leg exactly. Group by (extent, block) so
    // each block is read once.
    let manifest_by_id: std::collections::HashMap<_, _> =
        extents.iter().map(|m| (m.id, m)).collect();
    // Column ids to project (positions in the table schema).
    let proj: Vec<kyma_core::segment_format::ColumnId> = non_vector_cols
        .iter()
        .filter_map(|name| {
            table
                .schema
                .fields()
                .iter()
                .position(|f| f.name() == name)
                .map(|i| kyma_core::segment_format::ColumnId(i as u32))
        })
        .collect();

    let mut by_block: std::collections::BTreeMap<(String, u32), Vec<(usize, f64)>> =
        std::collections::BTreeMap::new();
    for (i, h) in hits.iter().enumerate() {
        by_block
            .entry((h.extent_id.to_string(), h.addr.block.0))
            .or_default()
            .push((i, h.distance));
    }

    // hit-index → resolved JSON row (preserves ann_topk's ascending order).
    let mut resolved: Vec<Option<Value>> = vec![None; hits.len()];
    let mut readers: std::collections::HashMap<
        String,
        Arc<dyn kyma_core::segment_format::ExtentReader>,
    > = std::collections::HashMap::new();

    for ((extent_str, block), members) in by_block {
        let Some(hit0) = members.first().map(|(i, _)| &hits[*i]) else {
            continue;
        };
        let Some(manifest) = manifest_by_id.get(&hit0.extent_id) else {
            continue;
        };
        let reader = match readers.get(&extent_str) {
            Some(r) => r.clone(),
            None => {
                let r = format
                    .open_extent(kyma_core::segment_format::OpenExtentInput {
                        extent_id: manifest.id,
                        table_id: manifest.table_id,
                        schema: table.schema.clone(),
                        object_path: manifest.object_path.clone(),
                        byte_size: manifest.byte_size,
                    })
                    .await
                    .ok()?;
                readers.insert(extent_str.clone(), r.clone());
                r
            }
        };
        // Read the block projecting the non-vector columns (in table-schema
        // order, so JSON keys match the SQL leg).
        let batch = reader
            .read_block(kyma_core::segment_format::BlockId(block), &proj)
            .await
            .ok()?;
        for (hit_idx, dist) in members {
            let row = hits[hit_idx].addr.row as usize;
            if row >= batch.num_rows() {
                continue;
            }
            let one = batch.slice(row, 1);
            let mut rows = batch_to_json_rows(&one);
            if let Some(Value::Object(mut map)) = rows.pop() {
                map.insert("__score".to_string(), serde_json::json!(dist));
                resolved[hit_idx] = Some(Value::Object(map));
            }
        }
    }

    Some(resolved.into_iter().flatten().collect())
}

/// BM25 lexical leg over per-extent tantivy FTS sidecars. Returns `None` when
/// the table has no FTS sidecar (caller falls back to the token/LIKE path), or
/// `Some(rows)` ordered by BM25 relevance with a `__score`. Mirrors
/// [`ann_vector_rows`]: capability-gate on a sidecar, run `bm25_topk`, resolve
/// each hit's `(extent, block, row)` to the projected non-vector columns.
async fn bm25_lexical_rows(
    catalog: &Arc<dyn kyma_core::catalog::Catalog>,
    format: &Arc<dyn kyma_core::segment_format::SegmentFormat>,
    tenant: kyma_core::tenant::TenantId,
    table: &kyma_core::catalog::TableRef,
    query: &str,
    non_vector_cols: &[String],
) -> Option<Vec<Value>> {
    use kyma_core::index_sidecar::SidecarKind;
    if query.trim().is_empty() {
        return None;
    }
    let store = format.object_store()?;
    let extents = catalog
        .list_extents_in_tenant(
            tenant,
            table.id,
            table.current_snapshot_id,
            &kyma_core::catalog::PrunePredicate::default(),
        )
        .await
        .ok()?;
    if extents.is_empty() {
        return None;
    }
    let extent_ids: Vec<_> = extents.iter().map(|m| m.id).collect();
    let sidecars = catalog
        .list_index_sidecars(tenant, table.id, &extent_ids, Some(SidecarKind::TantivyFts))
        .await
        .ok()?;
    // The FTS-indexed column (first one with a sidecar). No sidecar → SQL path.
    let fts_col = sidecars.first().map(|d| d.column.clone())?;

    let cache = sidecar_cache();
    let hits = match kyma_exec::bm25_topk(
        catalog, tenant, &store, cache, table, &fts_col, query, PER_LEG_K, None,
    )
    .await
    {
        Ok(Some(h)) => h,
        // No coverage or a hard error → SQL fallback rather than dropping the leg.
        Ok(None) | Err(_) => return None,
    };

    let manifest_by_id: std::collections::HashMap<_, _> =
        extents.iter().map(|m| (m.id, m)).collect();
    let proj: Vec<kyma_core::segment_format::ColumnId> = non_vector_cols
        .iter()
        .filter_map(|name| {
            table
                .schema
                .fields()
                .iter()
                .position(|f| f.name() == name)
                .map(|i| kyma_core::segment_format::ColumnId(i as u32))
        })
        .collect();

    // Group by (extent, block); preserve BM25 order via the hit index.
    let mut by_block: std::collections::BTreeMap<(String, u32), Vec<(usize, f32)>> =
        std::collections::BTreeMap::new();
    for (i, h) in hits.iter().enumerate() {
        by_block
            .entry((h.extent_id.to_string(), h.addr.block.0))
            .or_default()
            .push((i, h.score));
    }
    let mut resolved: Vec<Option<Value>> = vec![None; hits.len()];
    let mut readers: std::collections::HashMap<
        String,
        Arc<dyn kyma_core::segment_format::ExtentReader>,
    > = std::collections::HashMap::new();
    for ((extent_str, block), members) in by_block {
        let Some(hit0) = members.first().map(|(i, _)| &hits[*i]) else {
            continue;
        };
        let Some(manifest) = manifest_by_id.get(&hit0.extent_id) else {
            continue;
        };
        let reader = match readers.get(&extent_str) {
            Some(r) => r.clone(),
            None => {
                let r = format
                    .open_extent(kyma_core::segment_format::OpenExtentInput {
                        extent_id: manifest.id,
                        table_id: manifest.table_id,
                        schema: table.schema.clone(),
                        object_path: manifest.object_path.clone(),
                        byte_size: manifest.byte_size,
                    })
                    .await
                    .ok()?;
                readers.insert(extent_str.clone(), r.clone());
                r
            }
        };
        let batch = reader
            .read_block(kyma_core::segment_format::BlockId(block), &proj)
            .await
            .ok()?;
        for (hit_idx, score) in members {
            let row = hits[hit_idx].addr.row as usize;
            if row >= batch.num_rows() {
                continue;
            }
            let one = batch.slice(row, 1);
            let mut rows = batch_to_json_rows(&one);
            if let Some(Value::Object(mut map)) = rows.pop() {
                map.insert("__score".to_string(), serde_json::json!(score));
                resolved[hit_idx] = Some(Value::Object(map));
            }
        }
    }
    // bm25_topk already returns hits in descending BM25 order; keep it (the RRF
    // fusion ranks by position).
    Some(resolved.into_iter().flatten().collect())
}

/// Process-shared sidecar disk cache for the ANN query path.
fn sidecar_cache() -> &'static kyma_storage::sidecar_cache::SidecarCache {
    static CACHE: OnceLock<kyma_storage::sidecar_cache::SidecarCache> = OnceLock::new();
    CACHE.get_or_init(kyma_storage::sidecar_cache::SidecarCache::from_env)
}

/// Convert a single-row `RecordBatch` slice to JSON objects (best-effort).
fn batch_to_json_rows(batch: &arrow::record_batch::RecordBatch) -> Vec<Value> {
    let mut bytes: Vec<u8> = Vec::new();
    {
        let mut w = ArrayWriter::new(&mut bytes);
        if w.write(batch).is_err() || w.finish().is_err() {
            return Vec::new();
        }
    }
    let mut out: Vec<Value> = Vec::new();
    let stream = serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>();
    for arr in stream.flatten() {
        match arr {
            Value::Array(rows) => out.extend(rows),
            other => out.push(other),
        }
    }
    out
}

// ── helpers (pure, unit-tested) ───────────────────────────────────────────────

fn is_vector_type(dt: &DataType) -> bool {
    match dt {
        DataType::FixedSizeList(field, _) | DataType::List(field) | DataType::LargeList(field) => {
            matches!(
                field.data_type(),
                DataType::Float16 | DataType::Float32 | DataType::Float64
            )
        }
        _ => false,
    }
}

fn vector_column(table: &kyma_core::catalog::TableRef) -> Option<String> {
    table
        .schema
        .fields()
        .iter()
        .find(|f| is_vector_type(f.data_type()))
        .map(|f| f.name().clone())
}

/// Render an embedding as a DataFusion `make_array(...)` literal.
fn make_array(embedding: &[f32]) -> String {
    let mut s = String::with_capacity(embedding.len() * 8 + 16);
    s.push_str("make_array(");
    for (i, x) in embedding.iter().enumerate() {
        if i > 0 {
            s.push_str(", ");
        }
        // Cast to REAL so DataFusion infers a Float32 list matching the column.
        s.push_str(&format!("CAST({x} AS REAL)"));
    }
    s.push(')');
    s
}

fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn strip(row: &mut Value, key: &str) {
    if let Value::Object(map) = row {
        map.remove(key);
    }
}

/// Stable content key for a row, independent of column order, for RRF dedup.
fn row_key(row: &Value) -> String {
    match row {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, &Value> = map.iter().collect();
            serde_json::to_string(&sorted).unwrap_or_default()
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_schema::{Field, Schema as ArrowSchema, TimeUnit};
    use serde_json::json;

    #[test]
    fn make_array_renders_float_literal() {
        assert_eq!(
            make_array(&[0.1, 0.2]),
            "make_array(CAST(0.1 AS REAL), CAST(0.2 AS REAL))"
        );
    }

    #[test]
    fn apply_rerank_scores_reorders_by_score_and_caps() {
        // RRF order is a, b, c; the reranker disagrees: c best, then a, then b.
        let head = vec![
            ("s".into(), 0.9, json!({"id": "a"})),
            ("s".into(), 0.8, json!({"id": "b"})),
            ("s".into(), 0.7, json!({"id": "c"})),
        ];
        let out = apply_rerank_scores(head, vec![0.2, 0.1, 0.95], 2);
        assert_eq!(out.len(), 2, "truncated to limit");
        assert_eq!(out[0].2["id"], "c", "highest rerank score first");
        assert_eq!(out[1].2["id"], "a");
        // The output score is the rerank score, not the old RRF score.
        assert!((out[0].1 - 0.95).abs() < 1e-6);
    }

    #[test]
    fn row_text_joins_strings_skips_internal_and_bounds() {
        let row = json!({
            "msg": "connection pool exhausted",
            "service": "api",
            "__score": 0.5,
            "count": 7,
        });
        let t = row_text(&row);
        assert!(t.contains("connection pool exhausted"));
        assert!(t.contains("api"));
        assert!(!t.contains("0.5"), "internal __ fields excluded");
        assert!(!t.contains('7'), "non-string fields excluded");

        // Bounded to 2000 chars on a char boundary.
        let big = json!({ "body": "x".repeat(5000) });
        assert!(row_text(&big).len() <= 2000);
    }

    #[test]
    fn row_key_is_order_independent() {
        let a = json!({ "ts": "x", "msg": "y" });
        let b = json!({ "msg": "y", "ts": "x" });
        assert_eq!(row_key(&a), row_key(&b));
        let c = json!({ "msg": "z", "ts": "x" });
        assert_ne!(row_key(&a), row_key(&c));
    }

    #[test]
    fn vector_column_detected_by_type() {
        let fields = vec![
            Field::new("ts", DataType::Timestamp(TimeUnit::Microsecond, None), true),
            Field::new("msg", DataType::Utf8, true),
            Field::new(
                "embedding",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 384),
                true,
            ),
        ];
        let schema = Arc::new(ArrowSchema::new(fields));
        assert!(is_vector_type(schema.field(2).data_type()));
        assert!(!is_vector_type(schema.field(1).data_type()));
    }

    #[test]
    fn strip_removes_score() {
        let mut row = json!({ "a": 1, "__score": 0.5 });
        strip(&mut row, "__score");
        assert_eq!(row, json!({ "a": 1 }));
    }
}
