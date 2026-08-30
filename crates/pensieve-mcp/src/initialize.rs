//! Handler for the MCP `initialize` method.
//!
//! Wire spec: <https://modelcontextprotocol.io/specification/2025-03-26/basic/lifecycle#initialization>.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::jsonrpc::{ErrorCode, ErrorObject};

/// MCP protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "2025-03-26";

#[derive(Debug, Clone, Serialize)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
    /// The connecting client's advertised identity (e.g. claude-code).
    #[serde(rename = "clientInfo", default)]
    client_info: Option<ClientInfo>,
}

#[derive(Debug, Deserialize)]
struct ClientInfo {
    name: String,
    #[serde(default)]
    version: String,
}

pub fn handle_initialize(params: Value, server: &ServerInfo) -> Result<Value, ErrorObject> {
    let parsed: InitializeParams = serde_json::from_value(params).map_err(|e| {
        ErrorObject::new(ErrorCode::InvalidParams as i64, format!("initialize: {e}"))
    })?;

    // Record who is connecting — stamped into the provenance of every memory
    // this client writes (see `pensieve_server::agent::identity`).
    if let Some(ci) = &parsed.client_info {
        pensieve_server::agent::identity::record_client(&ci.name, &ci.version);
    }

    // Echo client's version if matched, otherwise advertise our own.
    let echoed = if parsed.protocol_version == PROTOCOL_VERSION {
        parsed.protocol_version
    } else {
        PROTOCOL_VERSION.to_string()
    };

    Ok(json!({
        "protocolVersion": echoed,
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": server.name,
            "version": server.version,
        }
    }))
}
