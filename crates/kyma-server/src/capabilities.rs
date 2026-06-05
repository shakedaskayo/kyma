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
    };
}

/// Build the capabilities router — caller wraps with auth middleware.
pub fn router(caps: Capabilities) -> Router {
    Router::new().route("/v1/capabilities", get(move || async move { Json(caps) }))
}
