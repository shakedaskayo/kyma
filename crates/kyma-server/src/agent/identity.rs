//! Writer identity: *who* is producing memory writes.
//!
//! Every memory save stamps a `writer` block into its provenance —
//! `{host, source, client, client_version}` — so curation, conflict
//! resolution, and the DLQ can tell a claude-code session on one machine from
//! a cursor session on another, and future consistency policies can key off
//! the writing client.
//!
//! - `host` is resolved once per process.
//! - `source` is the transport/entry point, set once by the binary at startup
//!   (`mcp-stdio`, `local-serve`, `server`, …).
//! - `client` is the MCP client's advertised `clientInfo` (e.g. claude-code),
//!   recorded at `initialize`. Exact for stdio (one client per process);
//!   best-effort latest-wins for the HTTP transport, where concurrent
//!   clients share the process.

use serde_json::{json, Value};
use std::sync::{OnceLock, RwLock};

static HOST: OnceLock<String> = OnceLock::new();
static SOURCE: OnceLock<String> = OnceLock::new();
static CLIENT: RwLock<Option<(String, String)>> = RwLock::new(None);

/// Set the process's write source once (first call wins): the transport or
/// entry point memories are produced through.
pub fn set_source(source: &str) {
    let _ = SOURCE.set(source.to_string());
}

/// Record the MCP client identity from an `initialize` request.
pub fn record_client(name: &str, version: &str) {
    if let Ok(mut slot) = CLIENT.write() {
        *slot = Some((name.to_string(), version.to_string()));
    }
}

fn host() -> &'static str {
    HOST.get_or_init(|| {
        std::env::var("HOSTNAME")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .or_else(|| {
                std::process::Command::new("hostname")
                    .output()
                    .ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                    .filter(|h| !h.is_empty())
            })
            .unwrap_or_else(|| "unknown".to_string())
    })
}

/// The current writer identity as a provenance block.
pub fn writer_json() -> Value {
    let client = CLIENT.read().ok().and_then(|c| c.clone());
    json!({
        "host": host(),
        "source": SOURCE.get().map(String::as_str).unwrap_or("unknown"),
        "client": client.as_ref().map(|(n, _)| n.clone()),
        "client_version": client.as_ref().map(|(_, v)| v.clone()),
    })
}

/// Merge the writer identity into a memory's provenance (under `writer`),
/// preserving whatever provenance the caller already set.
pub fn stamp_provenance(cm: &mut kyma_memory::CreateMemory) {
    let writer = writer_json();
    match &mut cm.provenance {
        Some(Value::Object(obj)) => {
            obj.insert("writer".to_string(), writer);
        }
        Some(other) => {
            // Non-object provenance: wrap rather than clobber.
            cm.provenance = Some(json!({"value": other.clone(), "writer": writer}));
        }
        None => cm.provenance = Some(json!({"writer": writer})),
    }
}
