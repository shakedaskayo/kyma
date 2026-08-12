//! JSON-RPC 2.0 Model Context Protocol server for kyma.
//!
//! Wraps the agent query + Agentic Memory tools from `kyma_server::agent`
//! as MCP tools, served over **Streamable HTTP** at `/mcp/v1` (`router`) and
//! over **stdio** (`stdio`) for the local single-binary mode (`kyma local
//! --mcp`). Both transports share the transport-agnostic `dispatch` layer.
//! Wire spec: <https://modelcontextprotocol.io/>.

#![forbid(unsafe_code)]

pub mod jsonrpc;

#[cfg(test)]
mod jsonrpc_unit_tests;

pub mod initialize;

#[cfg(test)]
mod initialize_unit_tests;

pub mod tools;

#[cfg(test)]
mod tools_unit_tests;

pub mod dispatch;

pub mod router;

#[cfg(test)]
mod router_unit_tests;

pub mod stdio;

pub use dispatch::dispatch_request;
pub use initialize::ServerInfo;
pub use router::{router, McpState};
pub use stdio::{serve as serve_stdio_loop, serve_stdio};
pub use tools::{DispatchBuilder, ToolDispatch};
