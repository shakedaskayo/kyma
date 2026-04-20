//! HTTP / NDJSON ingest frontend.
//!
//! Accepts `POST /v1/ingest` with headers:
//!   - `X-Database`: target database (default `default`)
//!   - `X-Table`: target table name (required)
//!   - `Content-Type: application/x-ndjson`
//!
//! Body is NDJSON — one JSON object per line. The table's catalog-stored
//! schema is used to coerce fields; unknown fields are dropped (phase A —
//! dynamic-column routing lands in M2).
//!
//! Response (200 JSON):
//! ```json
//! { "snapshot_id": "...", "rows_ingested": 123, "bytes_written": 4567 }
//! ```

#![forbid(unsafe_code)]

use arrow::json::ReaderBuilder;
use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderName, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use bytes::Bytes;
use kyma_core::catalog::Catalog;
use kyma_ingest_core::{IngestAck, WritePath};
use serde::Serialize;
use std::io::Cursor;
use std::sync::Arc;
use tower_http::request_id::{
    MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer,
};
use tracing::{error, info};

const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Shared HTTP-handler state.
#[derive(Clone)]
pub struct IngestState {
    pub catalog: Arc<dyn Catalog>,
    pub write_path: WritePath,
}

/// Build the ingest router. Mount at whatever base path the server wants.
pub fn router(state: IngestState) -> Router {
    Router::new()
        .route("/v1/ingest", post(ingest_handler))
        .with_state(state)
        // Set an X-Request-ID if the client didn't send one, then propagate
        // it back on the response so clients and logs share the same id.
        .layer(SetRequestIdLayer::new(REQUEST_ID_HEADER.clone(), MakeRequestUuid))
        .layer(PropagateRequestIdLayer::new(REQUEST_ID_HEADER.clone()))
}

/// Response shape returned on successful ingest.
#[derive(Debug, Serialize)]
pub struct IngestResponse {
    pub snapshot_id: String,
    pub extent_count: usize,
    pub rows_ingested: u64,
    pub bytes_written: u64,
    /// `true` if this response was replayed from the idempotency ledger.
    pub replayed: bool,
}

impl From<IngestAck> for IngestResponse {
    fn from(a: IngestAck) -> Self {
        Self {
            snapshot_id: a.snapshot_id.to_string(),
            extent_count: a.extent_count,
            rows_ingested: a.rows_ingested,
            bytes_written: a.bytes_written,
            replayed: a.replayed,
        }
    }
}

async fn ingest_handler(
    State(state): State<IngestState>,
    req: Request,
) -> Response {
    let (parts, body) = req.into_parts();
    let headers: &HeaderMap = &parts.headers;
    let request_id = extract_request_id(headers);

    let database = headers
        .get("x-database")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default")
        .to_owned();
    let idempotency_key: Option<String> = headers
        .get("x-idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned());
    let table = match headers.get("x-table").and_then(|v| v.to_str().ok()) {
        Some(t) => t.to_owned(),
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "missing_table_header",
                "missing X-Table header",
                &request_id,
            );
        }
    };

    // Actually read the body now that we have what we need from headers.
    let body: Bytes = match axum::body::to_bytes(body, 64 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                &format!("failed to read request body: {e}"),
                &request_id,
            );
        }
    };

    let table_ref = match state.catalog.lookup_table(&database, &table).await {
        Ok(t) => t,
        Err(e) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "table_not_found",
                &format!("table lookup failed: {e}"),
                &request_id,
            );
        }
    };

    // Parse the NDJSON body into `RecordBatch`es using the table's schema.
    let cursor = Cursor::new(body.to_vec());
    let reader = match ReaderBuilder::new(table_ref.schema.clone()).build(cursor) {
        Ok(r) => r,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "bad_request_body",
                &format!("failed to build NDJSON reader: {e}"),
                &request_id,
            );
        }
    };

    let mut batches: Vec<arrow_array::RecordBatch> = Vec::new();
    for b in reader {
        match b {
            Ok(batch) => batches.push(batch),
            Err(e) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "bad_request_body",
                    &format!("failed to decode NDJSON: {e}"),
                    &request_id,
                );
            }
        }
    }

    match state
        .write_path
        .ingest_with_idempotency(&table_ref, batches, idempotency_key.as_deref())
        .await
    {
        Ok(ack) => {
            info!(
                request_id = %request_id,
                database = %database,
                table = %table,
                snapshot_id = %ack.snapshot_id,
                rows = ack.rows_ingested,
                bytes = ack.bytes_written,
                "ingest committed"
            );
            let resp: IngestResponse = ack.into();
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            error!(request_id = %request_id, database = %database, table = %table, error = %e, "ingest failed");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "ingest_failed",
                &format!("{e}"),
                &request_id,
            )
        }
    }
}

// ---- shared error-body shape -----------------------------------------

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    code: &'a str,
    message: &'a str,
    request_id: &'a str,
}

fn error_response(
    status: StatusCode,
    code: &str,
    message: &str,
    request_id: &str,
) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorDetail {
                code,
                message,
                request_id,
            },
        }),
    )
        .into_response()
}

fn extract_request_id(headers: &HeaderMap) -> String {
    headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}
