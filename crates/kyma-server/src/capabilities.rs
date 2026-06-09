//! `/v1/capabilities` — feature discovery for clients.
//!
//! Local mode (`kyma serve`, embedded SQLite) deliberately omits the
//! control-plane surfaces (connectors, credentials, OAuth, saved Discover
//! views). The web UI gates those pages on these flags so it can explain
//! "runs on the control plane" instead of discovering missing routes via
//! 404s. Servers that predate this endpoint 404 here too — clients should
//! treat that as "assume everything" (the hosted server has it all).

use axum::{routing::get, Json, Router};
use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
pub struct Capabilities {
    /// `"local"` (embedded SQLite, single binary) or `"server"` (control plane).
    pub mode: &'static str,
    pub connectors: bool,
    pub credentials: bool,
    pub oauth: bool,
    /// Saved Discover views (write side is Postgres-backed today).
    pub saved_views: bool,
    pub users_admin: bool,
    /// Live-tail WebSocket (`GET /v1/explore/live`).
    pub explore_live: bool,
    /// Inline agent surface (`POST /v1/agent/ask` + sessions/skills/memory).
    /// Embedded clients gate the chat component on this so servers without
    /// the agent degrade to a "not available" card instead of a 404.
    pub agent: bool,
}

impl Capabilities {
    /// Hosted control plane — everything on.
    pub const SERVER: Self = Self {
        mode: "server",
        connectors: true,
        credentials: true,
        oauth: true,
        saved_views: true,
        users_admin: true,
        explore_live: true,
        agent: true,
    };
    /// Local single binary — memory + data + graph + dashboards; connector
    /// and credential management live on the control plane.
    pub const LOCAL: Self = Self {
        mode: "local",
        connectors: false,
        credentials: false,
        oauth: false,
        saved_views: false,
        users_admin: true,
        explore_live: true,
        agent: true,
    };
}

/// Build the capabilities router — caller wraps with auth middleware.
pub fn router(caps: Capabilities) -> Router {
    Router::new().route("/v1/capabilities", get(move || async move { Json(caps) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_serialize_explore_live() {
        let json = serde_json::to_value(Capabilities::SERVER).unwrap();
        assert_eq!(json["explore_live"], true);
        let json = serde_json::to_value(Capabilities::LOCAL).unwrap();
        assert_eq!(json["explore_live"], true);
    }

    #[test]
    fn capabilities_serialize_agent() {
        let json = serde_json::to_value(Capabilities::SERVER).unwrap();
        assert_eq!(json["agent"], true);
        let json = serde_json::to_value(Capabilities::LOCAL).unwrap();
        assert_eq!(json["agent"], true);
    }
}
