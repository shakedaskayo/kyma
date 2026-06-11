//! First-class property-graph layer for kyma: wire types, the `GraphProvider`
//! trait, and the synthetic schema-graph provider (catalog → property-graph).
//!
//! This crate is intentionally decoupled from `kyma-core`: it consumes a
//! narrow [`SchemaSource`] trait rather than the full catalog, so providers
//! are unit-testable without a database.

pub mod executor;
pub mod layout;
pub mod provider;
pub mod schema_graph;
pub mod source;
pub mod stored_graph;
pub mod types;

pub use executor::{GraphQueryExecutor, JsonRow, StoredGraphConfig};
pub use layout::{compute_layout, LayoutAlgorithm, LAYOUT_HEIGHT, LAYOUT_WIDTH};
pub use provider::GraphProvider;
pub use schema_graph::SchemaGraphProvider;
pub use stored_graph::StoredGraphProvider;
pub use source::{ColumnDef, SchemaSource};
pub use types::{
    Direction, EdgeExpansion, GraphExportPage, GraphNode, GraphPayload, GraphRef,
    GraphRelationship, GraphSchema, GraphStats, NodeMetadata, PositionedNode, Props, SearchHits,
};
