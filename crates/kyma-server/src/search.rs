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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::task::JoinSet;

use crate::discover::compile::{compile_for_source, TimeRange};
use crate::discover::grammar::Clause;
use crate::discover::scope::{resolve as resolve_scope, ResolvedSource, Scope};
use crate::QueryState;

/// Reciprocal-rank-fusion constant in `1 / (RRF_K + rank)`.
const RRF_K: f64 = 60.0;
const PER_LEG_K: usize = 50;
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;
const DEFAULT_MAX_SOURCES: usize = 200;
const MAX_BODY_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize)]
pub struct SearchBody {
    #[serde(default)]
    pub query: String,
    pub scope: Scope,
    #[serde(default)]
    pub time_range: Option<crate::discover::handler::TimeRangeBody>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    /// `db.table` the row came from.
    pub source: String,
    /// Fused RRF score (higher = more relevant).
    pub score: f64,
    pub row: Value,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub sources_searched: usize,
    pub elapsed_ms: u64,
}

pub async fn search_handler(State(state): State<QueryState>, req: Request<Body>) -> Response {
    let start = Instant::now();
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
    let payload: SearchBody = match serde_json::from_slice(&body_bytes) {
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

    let limit = payload.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let time_range = match payload.time_range.as_ref() {
        None => None,
        Some(tr) => match crate::discover::handler::parse_time_range(tr) {
            Ok(t) => Some(t),
            Err(msg) => {
                return crate::error_response(
                    StatusCode::BAD_REQUEST,
                    "bad_time_range",
                    &msg,
                    &request_id,
                )
            }
        },
    };

    // Principal → tenant + RBAC db filter (mirrors discover/query handlers).
    let principal = parts.extensions.get::<crate::auth::Principal>();
    let tenant = principal
        .map(|p| p.tenant)
        .unwrap_or(kyma_core::tenant::DEFAULT_TENANT);

    let max_sources = std::env::var("KYMA_DISCOVER_MAX_SOURCES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_SOURCES);

    let mut sources = match resolve_scope(&payload.scope, tenant, state.catalog.clone(), None, max_sources).await {
        Ok(s) => s,
        Err(e) => {
            return crate::error_response(
                StatusCode::BAD_REQUEST,
                "scope_error",
                &format!("{e}"),
                &request_id,
            )
        }
    };
    if let Some(principal) = principal {
        if let Some(allowed) = &principal.allowed_databases {
            sources.retain(|s| allowed.iter().any(|a| a == &s.db));
        }
    }

    // Embed the query once (process-shared model). Lexical-only if unavailable.
    let qvec: Option<Vec<f32>> = if payload.query.trim().is_empty() {
        None
    } else {
        match kyma_memory::shared_embedding().await {
            Ok(embedder) => embedder
                .embed(std::slice::from_ref(&payload.query))
                .await
                .ok()
                .and_then(|mut v| v.drain(..).next()),
            Err(_) => None,
        }
    };

    let sources_searched = sources.len();

    // Fan out per source.
    let mut set: JoinSet<Vec<(String, f64, Value)>> = JoinSet::new();
    for src in sources {
        let catalog = state.catalog.clone();
        let format = state.format.clone();
        let node_id = state.node_id;
        let query = payload.query.clone();
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

    let resp = SearchResponse {
        hits: hits
            .into_iter()
            .map(|(source, score, row)| SearchHit { source, score, row })
            .collect(),
        sources_searched,
        elapsed_ms: start.elapsed().as_millis() as u64,
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

    // Fresh, modestly-bounded context.
    let runtime = match RuntimeEnvBuilder::new()
        .with_memory_pool(Arc::new(GreedyMemoryPool::new(256 * 1024 * 1024)))
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
    let vector_rows = match (vec_col, qvec) {
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
    let mut fused: BTreeMap<String, (f64, Value)> = BTreeMap::new();
    for (rank, mut row) in lexical_rows.into_iter().enumerate() {
        strip(&mut row, "__score");
        let key = row_key(&row);
        let e = fused.entry(key).or_insert((0.0, row));
        e.0 += 1.0 / (RRF_K + rank as f64);
    }
    for (rank, mut row) in vector_rows.into_iter().enumerate() {
        strip(&mut row, "__score");
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
