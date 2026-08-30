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
use std::net::SocketAddr;
use std::sync::{OnceLock, RwLock};

static HOST: OnceLock<String> = OnceLock::new();
static SOURCE: OnceLock<String> = OnceLock::new();
static CLIENT: RwLock<Option<(String, String)>> = RwLock::new(None);
/// The connecting peer `(ip, best-effort local pid)`, recorded at MCP
/// `initialize` and the HTTP recall handler. Process-global latest-wins (the
/// HTTP transport shares one process); exact for single-user local mode.
static PEER: RwLock<Option<(String, Option<u32>)>> = RwLock::new(None);

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

/// Record the connecting peer's address. `resolve_pid` triggers a best-effort
/// local socket→process lookup (loopback only) — do this sparingly (at MCP
/// `initialize`), NOT on every request, since it shells out to `lsof`.
pub fn record_peer(addr: SocketAddr, resolve_pid: bool) {
    let ip = addr.ip().to_string();
    let pid = if resolve_pid {
        resolve_local_pid(&addr)
    } else {
        None
    };
    if let Ok(mut slot) = PEER.write() {
        // Keep an existing pid if this call didn't resolve one (e.g. a recall
        // request after the initialize already resolved it for this session).
        let keep_pid = match (&*slot, pid) {
            (Some((_, prev)), None) => *prev,
            (_, p) => p,
        };
        *slot = Some((ip, keep_pid));
    }
}

/// Best-effort: the local process that owns a loopback peer socket. Returns
/// `None` for non-loopback peers, or when `lsof` is unavailable / unparseable.
fn resolve_local_pid(addr: &SocketAddr) -> Option<u32> {
    if !addr.ip().is_loopback() {
        return None;
    }
    let me = std::process::id();
    // `-iTCP:<port>` matches both ends of the loopback connection (the client
    // owns the ephemeral port locally; our server has it as a remote port), so
    // pick the pid that isn't us.
    let out = std::process::Command::new("lsof")
        .args([
            "-nP",
            &format!("-iTCP:{}", addr.port()),
            "-sTCP:ESTABLISHED",
            "-Fp",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            l.strip_prefix('p')
                .and_then(|p| p.trim().parse::<u32>().ok())
        })
        .find(|&pid| pid != me)
}

/// The recorded peer ip, if any.
pub fn peer_ip() -> Option<String> {
    PEER.read()
        .ok()
        .and_then(|s| s.as_ref().map(|(ip, _)| ip.clone()))
}

/// The recorded peer pid, if resolved.
pub fn peer_pid() -> Option<u32> {
    PEER.read()
        .ok()
        .and_then(|s| s.as_ref().and_then(|(_, pid)| *pid))
}

/// The MCP client's advertised version, if recorded.
pub fn client_version() -> Option<String> {
    CLIENT
        .read()
        .ok()
        .and_then(|c| c.as_ref().map(|(_, v)| v.clone()))
        .filter(|v| !v.is_empty())
}

/// The transport/source this process serves through (`local-serve`, `server`,
/// `mcp-stdio`), if set.
pub fn transport() -> Option<String> {
    SOURCE.get().cloned()
}

/// The server/host machine name.
pub fn host_name() -> String {
    host().to_string()
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

/// The writing client's advertised name (e.g. `claude-code`), or `None` when no
/// MCP client has identified itself. Used to attribute a memory's
/// `writer_agent_id` (S3.3) without threading a request principal everywhere.
pub fn client_name() -> Option<String> {
    CLIENT
        .read()
        .ok()
        .and_then(|c| c.as_ref().map(|(n, _)| n.clone()))
}

/// Best-effort consumer kind for live consumer-activity attribution: the MCP
/// client name when known (`claude-code`, `cursor`, …), else the transport
/// source (`local-serve`, `server`, `mcp-stdio`), else `"unknown"`. The graph
/// explorer overlay maps this to an icon + colour.
pub fn consumer_kind() -> String {
    client_name()
        .or_else(|| SOURCE.get().cloned())
        .unwrap_or_else(|| "unknown".to_string())
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
