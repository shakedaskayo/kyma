//! Primitive, opaque id types + shared type aliases.
//!
//! Ids are newtype wrappers around `uuid::Uuid`. They are intentionally
//! opaque — no cross-id comparisons, no hidden ordering. Construct with
//! `::new()` to generate a fresh v4 UUID.

use arrow_schema::Schema;
use std::sync::Arc;
use uuid::Uuid;

macro_rules! opaque_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generate a fresh v4 id.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Construct from a raw UUID (use sparingly; prefer `new`).
            pub const fn from_uuid(u: Uuid) -> Self {
                Self(u)
            }

            /// Borrow the inner UUID.
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

opaque_id!(
    DatabaseId,
    "Identifier for a logical database (namespace for tables)."
);
opaque_id!(TableId, "Identifier for a table.");
opaque_id!(
    SnapshotId,
    "Identifier for a table snapshot (catalog-level versioned view)."
);
opaque_id!(
    ExtentId,
    "Identifier for an extent (one immutable storage file)."
);
opaque_id!(
    SchemaSnapshotId,
    "Identifier for an immutable schema version."
);
opaque_id!(
    NodeId,
    "Identifier for a compute/ingest/query node in the cluster."
);

/// Shared reference to an Arrow schema.
pub type SchemaRef = Arc<Schema>;

/// Logical engine data type.
///
/// This is the engine-level (pensieve) view of a column type, distinct from
/// `arrow_schema::DataType` which describes the physical Arrow encoding.
/// Most primitive columns are represented directly by Arrow types; this
/// enum exists to carry extra semantic metadata that Arrow alone cannot
/// express — for example, a `Vector` column tracks its fixed dimension
/// and (optionally) the embedding model that produced it.
///
/// The `Vector` variant maps onto `arrow_schema::DataType::FixedSizeList`
/// of `Float32` at the Arrow layer; see Task A.7 for ingest coercion and
/// Task A.8 for distance UDFs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataType {
    /// Fixed-dimension `Float32` embedding vector.
    ///
    /// - `dimension`: the vector length, known at schema time.
    /// - `model_id`: optional identifier of the embedding model
    ///   (e.g. `"fastembed/bge-small-en-v1.5"`) — surfaced in DDL and
    ///   used by the planner to reject cross-model comparisons.
    Vector {
        dimension: u16,
        model_id: Option<String>,
    },
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataType::Vector {
                dimension,
                model_id,
            } => {
                write!(f, "vector({dimension})")?;
                if let Some(m) = model_id {
                    write!(f, " MODEL '{m}'")?;
                }
                Ok(())
            }
        }
    }
}
