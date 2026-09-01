//! Graph differential-testing kit (S0.5).
//!
//! Everything the scale program's graph work is validated against lives here:
//!
//! - [`model`] — the minimal property-graph value type ([`model::TestGraph`])
//!   shared by generators, the oracle, and the ingest helper.
//! - [`synth`] — seeded, deterministic topology generators (power-law /
//!   grid / hub-and-spoke / cyclic). Same seed → byte-identical graph.
//! - [`oracle`] — the petgraph-backed reference engine. The doc comments on
//!   [`oracle::k_hop`], [`oracle::var_length`], [`oracle::shortest_path_len`]
//!   and [`oracle::match_chain`] are the *normative semantics* the S3
//!   vectorized path operator is implemented against.
//! - [`harness`] — engine-agnostic differential runner:
//!   [`harness::GraphEngineUnderTest`] + [`harness::run_differential`], with
//!   an [`harness::HttpPensieveEngine`] that drives a live pensieve over
//!   `POST /v1/query` (`application/x-cypher`).
//! - [`ingest`] — loads a [`model::TestGraph`] into a running pensieve (tables via
//!   `POST /v1/ingest` NDJSON, registration via `pensieve-cli create-graph`).
//!
//! The `graph_differential` binary (see `src/bin/`) ties these together into
//! the S0 gate: synth → ingest → run the existing Cypher subset through both
//! the engine and the oracle → fail on any set mismatch.

pub mod harness;
pub mod ingest;
pub mod model;
pub mod oracle;
pub mod synth;
