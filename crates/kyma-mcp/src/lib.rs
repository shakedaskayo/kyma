//! JSON-RPC 2.0 Model Context Protocol server for kyma.
//!
//! Wraps the eight agent tools from `kyma_server::agent::tools` as MCP
//! tools, served over Streamable HTTP at `/mcp/v1`. Wire spec:
//! <https://modelcontextprotocol.io/>.

#![forbid(unsafe_code)]

pub mod jsonrpc;

#[cfg(test)]
mod jsonrpc_unit_tests;
