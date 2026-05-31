//! Agentic Memory substrate for Kyma.
//!
//! Memory is stored as ordinary Kyma columnar tables (`memory_nodes` /
//! `memory_edges`) registered as the `memory` graph, so it is first-class
//! queryable (`run_sql`/`run_kql`/Discover) and renders in the unified
//! GraphView. Embeddings are a `FixedSizeList<Float32>` column searched via the
//! platform `cosine_distance` UDF. Storage is append-only; mutations write a new
//! version and recall dedups to the latest by `updated_at`.
//!
//! See `docs/superpowers/specs/2026-05-31-agentic-memory-design.md` (M1).

pub mod embed;
pub mod error;
pub mod rows;
pub mod schema;
pub mod sql;
pub mod types;
mod writer;

pub use embed::{build_embedding_backend, shared_embedding};
pub use error::{MemoryError, Result};
pub use types::{CreateMemory, MemoryStatus, MemoryType, RecallFilter};
pub use writer::MemoryWriter;

/// Dedicated database that holds the memory tables.
pub const DEFAULT_DATABASE: &str = "memory";
/// Registered graph name for memory.
pub const GRAPH_NAME: &str = "memory";
pub const NODE_TABLE: &str = "memory_nodes";
pub const EDGE_TABLE: &str = "memory_edges";

/// Default per-item realm when none is supplied.
pub const DEFAULT_REALM: &str = "default";
/// Shared realm for cross-project facts; recall unions project ∪ global.
pub const GLOBAL_REALM: &str = "global";

/// Recall re-rank blend: `score = RELEVANCE_WEIGHT*(1-distance) + IMPORTANCE_WEIGHT*importance`.
pub const RELEVANCE_WEIGHT: f64 = 0.7;
pub const IMPORTANCE_WEIGHT: f64 = 0.3;
