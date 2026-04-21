//! Inline agent surface — `POST /v1/agent/ask` + `GET /v1/agent/runs/:run_id`.
//!
//! Wires an ADK-Rust `Runner` backed by Ollama with four inline tools
//! (`list_databases`, `describe_table`, `run_sql`, `sample_rows`) and
//! streams SSE frames back to the caller. Each completed run is persisted
//! into the `agent_runs` table.
//!
//! See `docs/superpowers/specs/2026-04-21-nl-query-agent-and-vectors-design.md`
//! for the broader design context.

pub mod routes;
pub mod runner;
pub mod state;
pub mod tools;

pub use state::AgentState;

/// Build the `/v1/agent/*` sub-router. Caller typically mounts it under
/// `/v1/agent` and layers the same auth middleware as `/v1/query`.
pub fn router(state: AgentState) -> axum::Router {
    routes::router(state)
}
