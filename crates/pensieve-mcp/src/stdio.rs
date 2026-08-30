//! stdio transport for the MCP server — newline-delimited JSON-RPC.
//!
//! This is the local single-binary integration surface. A coding agent
//! (Claude Code, Cursor, Windsurf, …) spawns `kyma local --mcp` and speaks
//! JSON-RPC over the child's stdin/stdout: **one JSON message per line**,
//! responses **one per line**. The exact same [`dispatch_request`] powers the
//! HTTP transport, so the toolset and protocol are identical.
//!
//! **stdout is the protocol channel** — only JSON-RPC frames go there. All
//! logging must be routed to stderr by the host (`kyma local` configures the
//! tracing subscriber accordingly).

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::dispatch::dispatch_request;
use crate::initialize::ServerInfo;
use crate::jsonrpc::{parse_envelope, RequestEnvelope, Response};
use crate::router::McpState;
use crate::tools::ToolDispatch;

/// Serve the MCP protocol over the process's stdin/stdout until EOF.
pub async fn serve_stdio(state: McpState) -> std::io::Result<()> {
    serve(
        &state.dispatch,
        &state.server_info,
        tokio::io::stdin(),
        tokio::io::stdout(),
    )
    .await
}

/// Run the read → dispatch → write loop over arbitrary byte streams. Each input
/// line is one JSON-RPC request (or batch array); each non-notification yields
/// one response line. Returns when the reader hits EOF.
pub async fn serve<R, W>(
    dispatch: &ToolDispatch,
    server_info: &ServerInfo,
    reader: R,
    mut writer: W,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let out: Option<String> = match parse_envelope(trimmed.as_bytes()) {
            Ok(RequestEnvelope::Single(req)) => dispatch_request(dispatch, server_info, req)
                .await
                .map(|r| serde_json::to_string(&r).expect("response serializes")),
            Ok(RequestEnvelope::Batch(reqs)) => {
                let mut responses: Vec<Response> = Vec::with_capacity(reqs.len());
                for req in reqs {
                    if let Some(r) = dispatch_request(dispatch, server_info, req).await {
                        responses.push(r);
                    }
                }
                if responses.is_empty() {
                    None
                } else {
                    Some(serde_json::to_string(&responses).expect("batch serializes"))
                }
            }
            // Malformed input → a parse error with a null id (JSON-RPC 2.0).
            Err(err) => Some(
                serde_json::json!({ "jsonrpc": "2.0", "id": serde_json::Value::Null, "error": err })
                    .to_string(),
            ),
        };
        if let Some(msg) = out {
            writer.write_all(msg.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
