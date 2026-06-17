//! Axum router exposing the MCP server over Streamable HTTP.
//!
//! Endpoints:
//!   - `POST /mcp/v1` — JSON-RPC channel. Body is a single Request or a
//!     batch array. Notifications (no id) get HTTP 202 with empty body.
//!   - `GET /mcp/v1`  — SSE upgrade (Streamable HTTP). Slice 1a serves a
//!     minimal keepalive stream so MCP clients that probe SSE before
//!     falling back to POST get a valid handshake.
//!
//! Auth is NOT applied here — `kyma-bin` wraps the router with the
//! existing `require_role_middleware(Role::Read)` layer.

use axum::body::{Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures::stream::{self, Stream};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use crate::dispatch::dispatch_request;
use crate::initialize::ServerInfo;
use crate::jsonrpc::{
    parse_envelope, ErrorObject, Request as RpcRequest, RequestEnvelope, Response as RpcResponse,
};
use crate::tools::ToolDispatch;

#[derive(Clone)]
pub struct McpState {
    pub dispatch: ToolDispatch,
    pub server_info: ServerInfo,
}

/// Build the MCP router. Mounts `/mcp/v1` for both POST (JSON-RPC) and GET (SSE).
pub fn router(state: McpState) -> Router {
    Router::new()
        .route("/mcp/v1", post(handle_post).get(handle_get_sse))
        .with_state(state)
}

async fn handle_post(
    State(state): State<McpState>,
    // Optional so tests (which don't serve with connect-info) still work.
    peer: Option<ConnectInfo<SocketAddr>>,
    body: Bytes,
) -> Response {
    let envelope = match parse_envelope(&body) {
        Ok(env) => env,
        Err(err) => return parse_error_response_with_obj(err),
    };
    // Record the connecting peer for the live-consumers overlay. Resolve the
    // (best-effort, loopback-only) pid only on `initialize`, to amortize the
    // lsof cost across the session.
    if let Some(ConnectInfo(addr)) = peer {
        kyma_server::agent::identity::record_peer(addr, envelope_has_initialize(&envelope));
    }
    match envelope {
        RequestEnvelope::Single(req) => match dispatch_one(&state, req).await {
            Some(resp) => Json(resp).into_response(),
            None => StatusCode::ACCEPTED.into_response(),
        },
        RequestEnvelope::Batch(reqs) => {
            let mut out: Vec<RpcResponse> = Vec::with_capacity(reqs.len());
            for req in reqs {
                if let Some(resp) = dispatch_one(&state, req).await {
                    out.push(resp);
                }
            }
            if out.is_empty() {
                StatusCode::ACCEPTED.into_response()
            } else {
                Json(out).into_response()
            }
        }
    }
}

/// True when the envelope contains an `initialize` request (worth the pid lookup).
fn envelope_has_initialize(env: &RequestEnvelope) -> bool {
    match env {
        RequestEnvelope::Single(r) => r.method == "initialize",
        RequestEnvelope::Batch(rs) => rs.iter().any(|r| r.method == "initialize"),
    }
}

/// Dispatch a single JSON-RPC request. Returns `None` for notifications
/// (id absent) — caller emits HTTP 202. Delegates to the transport-agnostic
/// [`dispatch_request`] so HTTP and stdio share one protocol implementation.
async fn dispatch_one(state: &McpState, req: RpcRequest) -> Option<RpcResponse> {
    use tracing::Instrument;
    let span = tracing::info_span!("mcp.dispatch", method = %req.method);
    dispatch_request(&state.dispatch, &state.server_info, req)
        .instrument(span)
        .await
}

async fn handle_get_sse(
    State(_state): State<McpState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let s = stream::pending::<Result<Event, Infallible>>();
    Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

fn parse_error_response_with_obj(err: ErrorObject) -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": err,
    });
    let mut resp = Response::new(Body::from(body.to_string()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
}
