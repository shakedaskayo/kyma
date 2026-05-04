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
use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures::stream::{self, Stream};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::time::Duration;
use tracing::debug;

use crate::initialize::{handle_initialize, ServerInfo};
use crate::jsonrpc::{
    parse_envelope, ErrorCode, ErrorObject, Request as RpcRequest, RequestEnvelope,
    Response as RpcResponse,
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

async fn handle_post(State(state): State<McpState>, body: Bytes) -> Response {
    let envelope = match parse_envelope(&body) {
        Ok(env) => env,
        Err(err) => return parse_error_response_with_obj(err),
    };
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

/// Dispatch a single JSON-RPC request. Returns `None` for notifications
/// (id absent) — caller emits HTTP 202.
async fn dispatch_one(state: &McpState, req: RpcRequest) -> Option<RpcResponse> {
    use tracing::Instrument;
    let span = tracing::info_span!("mcp.dispatch", method = %req.method);
    dispatch_one_inner(state, req).instrument(span).await
}

async fn dispatch_one_inner(state: &McpState, req: RpcRequest) -> Option<RpcResponse> {
    let id = req.id.clone();
    let result: Result<Value, ErrorObject> = match req.method.as_str() {
        "initialize" => handle_initialize(req.params.unwrap_or(json!({})), &state.server_info),
        "notifications/initialized" => return None,
        "tools/list" => Ok(json!({ "tools": state.dispatch.list() })),
        "tools/call" => match req.params {
            Some(p) => {
                let name = p.get("name").and_then(|v| v.as_str()).map(str::to_owned);
                let arguments = p.get("arguments").cloned().unwrap_or(json!({}));
                match name {
                    Some(n) => state.dispatch.call(&n, arguments).await,
                    None => Err(ErrorObject::new(
                        ErrorCode::InvalidParams as i64,
                        "tools/call requires `name`",
                    )),
                }
            }
            None => Err(ErrorObject::new(
                ErrorCode::InvalidParams as i64,
                "tools/call requires params",
            )),
        },
        other => {
            debug!(method = %other, "mcp: method not found");
            Err(ErrorObject::new(
                ErrorCode::MethodNotFound as i64,
                format!("method not found: {other}"),
            ))
        }
    };
    match id {
        Some(id) => Some(match result {
            Ok(value) => RpcResponse::success(id, value),
            Err(err) => RpcResponse::error(id, err),
        }),
        None => None, // id-less request is a notification per JSON-RPC 2.0 — no response.
    }
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
