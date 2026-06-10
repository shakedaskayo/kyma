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
            search_one_source(src, &query, qv.as_deref(), tr, catalog, format, node_id).await
        });
    }

    let mut hits: Vec<(String, f64, Value)> = Vec::new();
    while let Some(joined) = set.join_next().await {
        if let Ok(rows) = joined {
            hits.extend(rows);
        }
    }

    // Global ranking by fused score.
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);
    hits
}

/// Run both legs for one source, RRF-fuse, return `(source_key, score, row)`.
async fn search_one_source(
    src: ResolvedSource,
    query: &str,
    qvec: Option<&[f32]>,
    time_range: Option<TimeRange>,
    catalog: Arc<dyn kyma_core::catalog::Catalog>,
    format: Arc<dyn kyma_core::segment_format::SegmentFormat>,
    node_id: Option<kyma_core::types::NodeId>,
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
            catalog,
            format,
            nid,
            src.db.clone(),
        )),
        None => Arc::new(KymaTable::new(src.table.clone(), catalog, format)),
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

    // ── Lexical leg (token-index `contains`) ──
    let lexical_rows = {
        let clauses: Vec<Clause> = if query.trim().is_empty() {
            Vec::new()
        } else {
            vec![Clause::Substring { value: query.to_string() }]
        };
        let compiled = compile_for_source(&src.table, &clauses, time_range.as_ref(), PER_LEG_K);
        match kyma_kql::kql_to_sql(&compiled.kql) {
            Ok(sql) => run_rows(&ctx, &sql).await,
            Err(_) => Vec::new(),
        }
    };

    // ── Vector leg (cosine_distance) ──
    let vector_rows = match (vec_col.as_deref(), qvec) {
        (Some(col), Some(qv)) if !non_vector_cols.is_empty() => {
            let proj = non_vector_cols
                .iter()
                .map(|c| quote_ident(c))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT {proj}, cosine_distance({vc}, {arr}) AS __score FROM {tbl} ORDER BY __score ASC LIMIT {k}",
                vc = quote_ident(&col),
                arr = make_array(qv),
                tbl = quote_ident(&table_name),
                k = PER_LEG_K,
            );
            run_rows(&ctx, &sql).await
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
        assert_eq!(make_array(&[0.1, 0.2]), "make_array(CAST(0.1 AS REAL), CAST(0.2 AS REAL))");
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
                DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, true)),
                    384,
                ),
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
