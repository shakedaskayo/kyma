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
use axum::{Extension, Json, Router};
use futures::stream::{self, Stream};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::time::Duration;

use crate::dispatch::dispatch_request;
use crate::initialize::ServerInfo;
use crate::jsonrpc::{
    parse_envelope, ErrorObject, Request as RpcRequest, RequestEnvelope, Response as RpcResponse,
};
use crate::tools::{DispatchBuilder, ToolDispatch};
use kyma_server::auth::{Principal, RealmScope};

#[derive(Clone)]
pub struct McpState {
    /// The startup-built, unrestricted dispatch — the fast path taken by every
    /// caller whose token carries no realm scope (the overwhelming majority).
    pub dispatch: ToolDispatch,
    /// Ingredients to rebuild a scoped dispatch per request for realm-restricted
    /// tokens. `None` in local/stdio mode, where there is no per-request
    /// `Principal`; a restricted token on such a transport is refused rather
    /// than silently served the unrestricted set.
    pub builder: Option<DispatchBuilder>,
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
    // Inserted into request extensions by `require_role_middleware` (kyma-bin
    // wraps the router with it). Optional so tests and the auth-disabled path
    // — which insert no principal — resolve to an unrestricted scope.
    principal: Option<Extension<Principal>>,
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

    // Resolve the per-request tool dispatch. Unrestricted tokens (the common
    // case) borrow the startup-built dispatch. A realm-restricted token gets a
    // freshly-built, scope-limited dispatch; if this transport has no builder
    // (local/stdio) it is refused — never silently served the full tool set.
    let scope = principal
        .as_ref()
        .map(|Extension(p)| RealmScope::from_principal(p))
        .unwrap_or_default();
    let dispatch: Cow<'_, ToolDispatch> = if scope.is_restricted() {
        match &state.builder {
            Some(b) => Cow::Owned(b.build(scope)),
            None => return realm_scope_unsupported_response(),
        }
    } else {
        Cow::Borrowed(&state.dispatch)
    };

    match envelope {
        RequestEnvelope::Single(req) => {
            match dispatch_one(&dispatch, &state.server_info, req).await {
                Some(resp) => Json(resp).into_response(),
                None => StatusCode::ACCEPTED.into_response(),
            }
        }
        RequestEnvelope::Batch(reqs) => {
            let mut out: Vec<RpcResponse> = Vec::with_capacity(reqs.len());
            for req in reqs {
                if let Some(resp) = dispatch_one(&dispatch, &state.server_info, req).await {
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

/// 501-style JSON-RPC error for a realm-scoped token on a transport that cannot
/// build a scoped dispatch (local/stdio). Fail closed.
fn realm_scope_unsupported_response() -> Response {
    let body = json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": {
            "code": -32001,
            "message": "realm-scoped tokens are not supported on this MCP transport",
        },
    });
    let mut resp = Response::new(Body::from(body.to_string()));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    resp
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
async fn dispatch_one(
    dispatch: &ToolDispatch,
    server_info: &ServerInfo,
    req: RpcRequest,
) -> Option<RpcResponse> {
    use tracing::Instrument;
    let span = tracing::info_span!("mcp.dispatch", method = %req.method);
    dispatch_request(dispatch, server_info, req)
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
