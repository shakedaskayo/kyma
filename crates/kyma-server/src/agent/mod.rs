//! Inline agent surface — `POST /v1/agent/ask` + `GET /v1/agent/runs/:run_id`.
//!
//! Wires an ADK-Rust `Runner` backed by Ollama with four inline tools
//! (`list_databases`, `describe_table`, `run_sql`, `sample_rows`) and
//! streams SSE frames back to the caller. Each completed run is persisted
//! into the `agent_runs` table.
//!
//! See `docs/superpowers/specs/2026-04-21-nl-query-agent-and-vectors-design.md`
//! for the broader design context.

pub mod cc_curate;
pub mod engine;
pub mod local;
pub mod memory;

#[cfg(test)]
mod cc_curate_unit_tests;
pub mod memory_conflict;
pub mod memory_extract;
pub mod memory_resolve;
pub mod memory_retrieve;
pub mod memory_settings;
pub mod memory_tools;
pub mod routes;
pub mod runner;
pub mod sessions;
pub mod skills;
pub mod state;
pub mod tools;
pub mod ui_stream;

pub use memory::MemoryConsolidator;
pub use memory_tools::{
    tool_ingest_entity, tool_link_memory_to_entity, tool_list_memories, tool_memory_compare,
    tool_memory_judge, tool_memory_search, tool_memory_session_summary, tool_recall_memory,
    tool_save_memory, tool_update_memory_importance, tool_update_memory_status,
};
pub use state::AgentState;
pub use tools::{
    execute_sql, tool_describe_table, tool_explore_schema, tool_find_references_to,
    tool_graph_traverse, tool_list_databases, tool_run_kql, tool_run_sql, tool_sample_rows,
    SharedToolCtx,
};

/// Build the `/v1/agent/*` sub-router. Caller typically mounts it under
/// `/v1/agent` and layers the same auth middleware as `/v1/query`.
pub fn router(state: AgentState) -> axum::Router {
    routes::router(state)
}
