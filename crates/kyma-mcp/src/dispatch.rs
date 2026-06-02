//! Transport-agnostic JSON-RPC method dispatch for the MCP server.
//!
//! Both transports — the HTTP Streamable router (`router.rs`) and the stdio
//! loop (`stdio.rs`) — call [`dispatch_request`], so they speak exactly the
//! same protocol. The only difference is framing (HTTP body vs newline-
//! delimited stdin/stdout).

use serde_json::{json, Value};
use tracing::debug;

use crate::initialize::{handle_initialize, ServerInfo};
use crate::jsonrpc::{ErrorCode, ErrorObject, Request, Response};
use crate::tools::ToolDispatch;

/// Dispatch a single JSON-RPC request against the tool dispatch + server info.
///
/// Returns `None` for notifications (id-less requests, e.g.
/// `notifications/initialized`) — the caller emits no response for those.
pub async fn dispatch_request(
    dispatch: &ToolDispatch,
    server_info: &ServerInfo,
    req: Request,
) -> Option<Response> {
    let id = req.id.clone();
    let result: Result<Value, ErrorObject> = match req.method.as_str() {
        "initialize" => handle_initialize(req.params.unwrap_or(json!({})), server_info),
        "notifications/initialized" => return None,
        "tools/list" => Ok(json!({ "tools": dispatch.list() })),
        "tools/call" => match req.params {
            Some(p) => {
                let name = p.get("name").and_then(|v| v.as_str()).map(str::to_owned);
                let arguments = p.get("arguments").cloned().unwrap_or(json!({}));
                match name {
                    Some(n) => {
                        ::metrics::counter!("kyma_mcp_tool_calls_total", "tool" => n.clone())
                            .increment(1);
                        let outcome = dispatch.call(&n, arguments).await;
                        let result_label = if outcome.is_ok() { "ok" } else { "error" };
                        ::metrics::counter!(
                            "kyma_mcp_tool_results_total",
                            "tool" => n.clone(),
                            "result" => result_label
                        )
                        .increment(1);
                        outcome
                    }
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
            Ok(value) => Response::success(id, value),
            Err(err) => Response::error(id, err),
        }),
        None => None, // id-less request is a notification per JSON-RPC 2.0.
    }
}
