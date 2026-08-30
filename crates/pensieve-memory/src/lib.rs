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

pub mod activities;
pub mod embed;
pub mod error;
pub mod file_candidates;
pub mod ingest;
pub mod reinforcement;
pub mod rerank;
pub mod rows;
pub mod schema;
pub mod sql;
pub mod types;
mod writer;

pub use embed::{
    build_embedding_backend, shared_embedding, shared_reranker, try_set_shared_embedding,
};
pub use error::{MemoryError, Result};
pub use ingest::{spawn_memory_queue, MemoryIngestConfig, MemoryOp, MemoryQueue};
pub use types::{CreateMemory, MemoryStatus, MemoryType, RecallFilter, SourceSummary};
pub use writer::{redact_create, MemoryWriter};

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

/// Canonical memory edge-relationship types.
///
/// - [`EDGE_REFERENCES`]: a memory → an entity/node it is about.
/// - [`EDGE_RESOLVES_TO`]: a memory-side `entity` node → the real catalog graph
///   node it denotes (carries `target_namespace` for cross-graph stitching).
/// - [`EDGE_RELATES_TO`]: entity ↔ entity, with the predicate in `props`.
/// - [`EDGE_DERIVED_FROM`]: a memory → the source firehose/data source row it was
///   extracted from (provenance edge). Also used memory → [`activities`] Activity
///   node (the raw input window it was extracted from, M8.2).
/// - [`EDGE_INVALIDATES`]: a new memory → the memory it supersedes (bi-temporal).
/// - [`EDGE_MERGED_INTO`]: a memory → the memory it was merged into.
/// - [`EDGE_GENERALIZES_FROM`]: an induced `procedure` memory (M8.4 schema
///   induction) → each supporting memory it was generalized from. Distinct
///   from `DERIVED_FROM`: that's raw-input provenance, this is abstraction
///   provenance (many supporting memories → one generalized pattern).
pub const EDGE_REFERENCES: &str = "REFERENCES";
pub const EDGE_RESOLVES_TO: &str = "RESOLVES_TO";
pub const EDGE_RELATES_TO: &str = "RELATES_TO";
pub const EDGE_DERIVED_FROM: &str = "DERIVED_FROM";
pub const EDGE_INVALIDATES: &str = "INVALIDATES";
pub const EDGE_MERGED_INTO: &str = "MERGED_INTO";
pub const EDGE_GENERALIZES_FROM: &str = "GENERALIZES_FROM";

/// Graph-aware hybrid recall blend. The final score fuses reciprocal-rank
/// fusion (RRF) of the vector + keyword candidate lists with semantic
/// similarity, keyword match, graph proximity, importance, and recency (see
/// the `memory_retrieve` orchestrator). Tunable; sane defaults below.
pub const W_RRF: f64 = 1.0;
pub const W_SEMANTIC: f64 = 0.6;
pub const W_KEYWORD: f64 = 0.3;
pub const W_GRAPH: f64 = 0.5;
pub const W_IMPORTANCE: f64 = 0.4;
pub const W_RECENCY: f64 = 0.3;
/// Reciprocal-rank-fusion constant in `1/(RRF_K + rank)`.
pub const RRF_K: f64 = 60.0;
/// Half-life (days) for recency decay `exp(-ln2 * age_days / HALF_LIFE_DAYS)`.
pub const HALF_LIFE_DAYS: f64 = 30.0;

/// Usage-based reinforcement blend weight (M8.1, see [`reinforcement`]).
/// Defaults to 0 — disabled — so enabling it is an explicit operator decision
/// and existing rankings never silently shift on upgrade.
pub const W_REINFORCEMENT: f64 = 0.0;
pub const REINFORCEMENT_HIT_WEIGHT: f64 = 0.3;
pub const REINFORCEMENT_MISS_PENALTY: f64 = 0.3;

/// Worked-example ("precedent") retrieval default (M8.2, see [`activities`]).
/// Much stricter than ordinary recall's semantic threshold — a precedent
/// means "near-duplicate past input," not "topically related."
pub const PRECEDENT_MAX_DISTANCE: f64 = 0.15;
