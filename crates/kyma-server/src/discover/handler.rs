//! Axum handler for `POST /v1/explore/search`.
//!
//! Wires: parse body → parse grammar → resolve scope → kick off fanout →
//! stream NDJSON response from the mpsc channel.
//!
//! Frame ordering is set by [`crate::discover::fanout`]: `Plan` first, then
//! per-source frames (interleaved), then `Done`. This handler does not
//! re-order; it only forwards bytes and aggregates terminal metrics.

use axum::body::{Body, Bytes};
use axum::extract::{Extension, State};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::response::Response;
use kyma_core::tenant::TenantId;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::compile::TimeRange;
use super::fanout::{run as run_fanout, FanoutInput};
use super::frames::{frame_to_line, Frame};
use super::grammar::parse as parse_grammar;
use super::scope::{resolve as resolve_scope, Scope, ScopeError};
use crate::QueryState;

const DEFAULT_PER_SOURCE_LIMIT: usize = 500;
const MAX_PER_SOURCE_LIMIT: usize = 5_000;
const DEFAULT_MAX_SOURCES: usize = 200;
const MAX_BODY_BYTES: usize = 1024 * 1024; // 1 MiB

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    #[serde(default)]
    pub query: String,
    pub scope: Scope,
    #[serde(default)]
    pub time_range: Option<TimeRangeBody>,
    #[serde(default)]
    pub per_source_limit: Option<usize>,
    #[serde(default)]
    pub histogram: Option<HistogramBody>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimeRangeBody {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct HistogramBody {
    pub interval_ms: u64,
}

/// Handler for `POST /v1/explore/search`. Streams NDJSON frames; never panics
/// on user input.
pub async fn discover_search_handler(
    State(state): State<QueryState>,
    Extension(tenant): Extension<TenantId>,
    req: Request<Body>,
) -> Response {
    let start = Instant::now();
    let (parts, body) = req.into_parts();
    let request_id = crate::extract_request_id(&parts.headers);

    // Count every request at entry so very-early failures (body-too-large,
    // bad JSON, scope_too_large, view_not_found) are visible in the
    // requests_total counter alongside the kyma_http_errors_total counter.
    ::metrics::counter!("kyma_explore_search_requests_total", "scope_kind" => "unknown")
        .increment(1);

    // 1. Read body (max 1 MiB).
    let body_bytes: Bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return crate::error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                &format!("failed to read body: {e}"),
                &request_id,
            );
        }
    };

    // 2. Parse JSON.
    let payload: SearchRequest = match serde_json::from_slice(&body_bytes) {
        Ok(p) => p,
        Err(e) => {
            return crate::error_response(
                StatusCode::BAD_REQUEST,
                "bad_request",
                &format!("invalid JSON body: {e}"),
                &request_id,
            );
        }
    };

    // 3. Clamp per-source limit to [1, 5000].
    let per_source_limit = payload
        .per_source_limit
        .unwrap_or(DEFAULT_PER_SOURCE_LIMIT)
        .clamp(1, MAX_PER_SOURCE_LIMIT);

    // 4. Parse the user search string into structured clauses.
    let clauses = match parse_grammar(&payload.query) {
        Ok(c) => c,
        Err(e) => {
            return crate::error_response(
                StatusCode::BAD_REQUEST,
                "grammar_error",
                &e.to_string(),
                &request_id,
            );
        }
    };

    // 5. Parse the time range (if present) and convert to millis.
    let time_range = match payload.time_range.as_ref() {
        None => None,
        Some(tr) => match parse_time_range(tr) {
            Ok(t) => Some(t),
            Err(msg) => {
                return crate::error_response(
                    StatusCode::BAD_REQUEST,
                    "bad_time_range",
                    &msg,
                    &request_id,
                );
            }
        },
    };

    // 6. Pull the scope-size cap from env (operator knob), defaulting to 200.
    let max_sources = std::env::var("KYMA_DISCOVER_MAX_SOURCES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_SOURCES);

    // 7. Resolve the scope under the caller's tenant.
    //
    //    Build a SavedViewLookup adapter from the request principal so
    //    `Scope::View` references resolve via the catalog's `saved_views`
    //    table. Auth-disabled deployments inject an admin Principal with
    //    `subject = None`; without a subject we have no owner column to
    //    scope the lookup by, so we pass `None` and `Scope::View` requests
    //    fall through to `ScopeError::ViewNotFound`.
    let subject_opt: Option<&str> = parts
        .extensions
        .get::<crate::auth::Principal>()
        .and_then(|p| p.subject.as_deref());

    // Saved-view lookup needs both an authenticated subject and a Postgres
    // pool. In local mode there is no pool, so named-view scopes fall through
    // to ViewNotFound (ad-hoc scopes are unaffected).
    let lookup: Option<crate::discover::saved_views_lookup::CatalogSavedViewLookup> =
        match (subject_opt, state.pg_pool.clone()) {
            (Some(s), Some(pool)) => {
                Some(crate::discover::saved_views_lookup::CatalogSavedViewLookup {
                    pool,
                    tenant_id: tenant.as_uuid(),
                    owner_subject: s.to_string(),
                })
            }
            _ => None,
        };

    let scope_kind = scope_kind_label(&payload.scope).to_string();
    let mut resolved = match resolve_scope(
        &payload.scope,
        tenant,
        state.catalog.clone(),
        lookup
            .as_ref()
            .map(|l| l as &(dyn crate::discover::scope::SavedViewLookup + Send + Sync)),
        max_sources,
    )
    .await
    {
        Ok(s) => s,
        Err(ScopeError::ScopeTooLarge(n, max)) => {
            return crate::error_response(
                StatusCode::BAD_REQUEST,
                "scope_too_large",
                &format!("scope resolves to {n} sources, exceeds max {max}"),
                &request_id,
            );
        }
        Err(ScopeError::ViewNotFound(id)) => {
            return crate::error_response(
                StatusCode::NOT_FOUND,
                "view_not_found",
                &format!("saved view {id} not found"),
                &request_id,
            );
        }
        Err(ScopeError::Catalog(msg)) => {
            return crate::error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "catalog_error",
                &msg,
                &request_id,
            );
        }
    };

    // Filter resolved sources to only those databases the principal may access.
    // Unrestricted principals (allowed_databases = None) pass everything through.
    if let Some(principal) = parts.extensions.get::<crate::auth::Principal>() {
        if let Some(allowed) = &principal.allowed_databases {
            resolved.retain(|src| allowed.iter().any(|a| a == &src.db));
        }
    }

    // 8. Resolve the per-request budget from headers (shared helper).
    let budget = crate::resolve_query_budget(&parts.headers);

    // 9. Spawn the fanout and stream frames over the mpsc.
    let (tx, rx) = mpsc::channel::<Frame>(64);

    // Now that scope_kind is known, count an executed request labeled with it.
    // The entry-counter above uses scope_kind="unknown" so it covers requests
    // that never made it past parsing.
    ::metrics::counter!(
        "kyma_explore_search_executed_total",
        "scope_kind" => scope_kind.clone()
    )
    .increment(1);

    let resolved_count = resolved.len();
    let req_id_for_log = request_id.clone();
    let scope_kind_for_stream = scope_kind.clone();

    run_fanout(
        FanoutInput {
            sources: resolved,
            clauses,
            time_range,
            per_source_limit,
            budget,
            catalog: state.catalog.clone(),
            format: state.format.clone(),
            node_id: state.node_id,
        },
        tx,
    );

    // 10. Wrap rx as a Stream, then convert each frame to a NDJSON line.
    //     The closing tail of the stream emits aggregate observability metrics
    //     and a single structured log line. Aggregation runs entirely inside
    //     the stream future so it observes channel closure naturally.
    let rx_stream = ReceiverStream::new(rx);
    let body_stream = async_stream::stream! {
        let mut total_rows: usize = 0;
        let mut error_count: usize = 0;
        let mut sources_done: usize = 0;
        let mut capped_sources: usize = 0;

        for await frame in rx_stream {
            match &frame {
                Frame::Rows { rows, .. } => total_rows += rows.len(),
                Frame::Error { .. } => error_count += 1,
                Frame::SourceDone { capped, .. } => {
                    sources_done += 1;
                    if *capped {
                        capped_sources += 1;
                    }
                }
                _ => {}
            }
            let line = frame_to_line(&frame);
            yield Ok::<_, Infallible>(Bytes::from(line));
        }

        let elapsed = start.elapsed();
        ::metrics::histogram!(
            "kyma_explore_search_duration_seconds",
            "scope_kind" => scope_kind_for_stream.clone()
        )
        .record(elapsed.as_secs_f64());
        ::metrics::histogram!(
            "kyma_explore_search_sources_resolved",
            "scope_kind" => scope_kind_for_stream.clone()
        )
        .record(resolved_count as f64);
        ::metrics::histogram!("kyma_explore_search_rows_returned")
            .record(total_rows as f64);
        ::metrics::counter!("kyma_explore_search_per_source_errors_total")
            .increment(error_count as u64);
        ::metrics::counter!("kyma_explore_search_cap_hits_total")
            .increment(capped_sources as u64);
        tracing::info!(
            request_id = %req_id_for_log,
            endpoint = "/v1/explore/search",
            scope_kind = %scope_kind_for_stream,
            scope_resolved_sources = resolved_count,
            sources_done,
            total_rows,
            capped_sources,
            error_count,
            elapsed_ms = elapsed.as_millis() as u64,
            "explore search completed"
        );
    };

    // 11. Build response with NDJSON content-type + propagated request id.
    let mut resp = Response::new(Body::from_stream(body_stream));
    let hdrs = resp.headers_mut();
    hdrs.insert(
        "content-type",
        HeaderValue::from_static("application/x-ndjson; charset=utf-8"),
    );
    if let Ok(rid) = HeaderValue::from_str(&request_id) {
        hdrs.insert("x-request-id", rid);
    }
    resp
}

fn scope_kind_label(s: &Scope) -> &'static str {
    match s {
        Scope::All => "all",
        Scope::Sources { .. } => "sources",
        Scope::View { .. } => "view",
    }
}

/// Parse a `{from, to}` RFC-3339 pair into a millis range. Rejects ranges
/// where `to <= from` so downstream compile logic can assume a positive
/// window.
pub(crate) fn parse_time_range(tr: &TimeRangeBody) -> Result<TimeRange, String> {
    let from = chrono::DateTime::parse_from_rfc3339(&tr.from)
        .map_err(|e| format!("from: {e}"))?
        .timestamp_millis();
    let to = chrono::DateTime::parse_from_rfc3339(&tr.to)
        .map_err(|e| format!("to: {e}"))?
        .timestamp_millis();
    if to <= from {
        return Err("time_range.to must be greater than time_range.from".into());
    }
    Ok(TimeRange {
        from_ms: from,
        to_ms: to,
    })
}
