//! First-class property-graph layer for kyma: wire types, the `GraphProvider`
//! trait, and the synthetic schema-graph provider (catalog → property-graph).
//!
//! This crate is intentionally decoupled from `kyma-core`: it consumes a
//! narrow [`SchemaSource`] trait rather than the full catalog, so providers
//! are unit-testable without a database.

pub mod provider;
pub mod schema_graph;
pub mod source;
pub mod types;

pub use provider::GraphProvider;
// TODO(G1a Task 5): re-export SchemaGraphProvider
// pub use schema_graph::SchemaGraphProvider;
pub use source::{ColumnDef, SchemaSource};
pub use types::*;
