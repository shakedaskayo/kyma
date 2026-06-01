//! JSON-RPC 2.0 Model Context Protocol server for kyma.
//!
//! Wraps the agent query + Agentic Memory tools from `kyma_server::agent`
//! as MCP tools, served over Streamable HTTP at `/mcp/v1`. Wire spec:
//! <https://modelcontextprotocol.io/>.

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

pub mod router;

#[cfg(test)]
mod router_unit_tests;

pub use initialize::ServerInfo;
pub use router::{router, McpState};
pub use tools::ToolDispatch;
