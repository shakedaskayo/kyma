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
