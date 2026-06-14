//! Arrow schemas for the columnar memory tables.
//!
//! `memory_nodes` / `memory_edges` are ordinary Kyma columnar tables, written
//! through the ingest `WritePath` and registered as the `memory` graph. They
//! must be created with an EXPLICIT schema (auto-provisioning can't declare the
//! `embedding` vector column). Timestamps are stored as RFC3339 strings so
//! lexicographic ordering = chronological ordering (and to avoid Arrow
//! timestamp-coercion pitfalls); `updated_at` is the latest-wins version key.

use std::sync::Arc;

use arrow_schema::{DataType, Field, Schema, SchemaRef};

/// Schema for `memory_nodes`. `dim` is the embedding dimension (e.g. 384).
///
/// Graph columns: `id` (node id), `labels` (node label), `realm` (namespace).
/// Everything else surfaces as graph node properties.
///
/// Bi-temporal validity (Zep/Graphiti style, invalidate-don't-delete):
/// `valid_at` is when the fact became true (defaults to `created_at`),
/// `invalid_at` is when it was superseded/contradicted (NULL = currently valid),
/// `superseded_by` points at the memory id that replaced it, and `provenance`
/// is a JSON blob describing how the memory was formed.
pub fn memory_nodes_schema(dim: i32) -> SchemaRef {
    let item = Arc::new(Field::new("item", DataType::Float32, false));
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("labels", DataType::Utf8, true),
        Field::new("realm", DataType::Utf8, true),
        Field::new("memory_type", DataType::Utf8, true),
        Field::new("title", DataType::Utf8, true),
        Field::new("content", DataType::Utf8, true),
        Field::new("content_preview", DataType::Utf8, true),
        Field::new("tags", DataType::Utf8, true),
        Field::new("importance", DataType::Float64, true),
        Field::new("status", DataType::Utf8, true),
        Field::new("source_session_id", DataType::Utf8, true),
        Field::new("source_run_id", DataType::Utf8, true),
        Field::new("embedding", DataType::FixedSizeList(item, dim), false),
        Field::new("created_at", DataType::Utf8, true),
        Field::new("updated_at", DataType::Utf8, true),
        Field::new("valid_at", DataType::Utf8, true),
        Field::new("invalid_at", DataType::Utf8, true),
        Field::new("superseded_by", DataType::Utf8, true),
        Field::new("provenance", DataType::Utf8, true),
        // Deterministic upsert key: (realm, topic_key) updates in place.
        Field::new("topic_key", DataType::Utf8, true),
        // Memory-class model (S3.3): retention/consolidation class, visibility
        // space (public / private:<agent>), and the writing agent.
        Field::new("memory_class", DataType::Utf8, true),
        Field::new("space", DataType::Utf8, true),
        Field::new("writer_agent_id", DataType::Utf8, true),
    ]))
}

/// Columns an older `memory_nodes` table may be missing (added after initial
/// provisioning: bi-temporal validity + the topic-key upsert key). The writer
/// detects the drift and backfills them via `alter_table_add_column` (old
/// extents null-fill on read).
pub const BITEMPORAL_COLUMNS: &[&str] =
    &["valid_at", "invalid_at", "superseded_by", "provenance", "topic_key"];

/// Memory-class columns (S3.3) added after bi-temporal support. Healed the same
/// way — nullable, so historical extents null-fill on read.
pub const MEMORY_CLASS_COLUMNS: &[&str] = &["memory_class", "space", "writer_agent_id"];

/// Schema for `memory_edges`. Graph columns: `src`, `dst`, `type`, `realm`.
/// `target_namespace` carries the foreign endpoint's `database/graph` for
/// cross-graph `REFERENCES` edges so the unified canvas can stitch them.
pub fn memory_edges_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("src", DataType::Utf8, true),
        Field::new("dst", DataType::Utf8, true),
        Field::new("type", DataType::Utf8, true),
        Field::new("realm", DataType::Utf8, true),
        Field::new("target_namespace", DataType::Utf8, true),
        Field::new("props", DataType::Utf8, true),
        Field::new("created_at", DataType::Utf8, true),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_schema_has_vector_embedding() {
        let s = memory_nodes_schema(384);
        let f = s.field_with_name("embedding").unwrap();
        match f.data_type() {
            DataType::FixedSizeList(inner, dim) => {
                assert_eq!(*dim, 384);
                assert_eq!(inner.data_type(), &DataType::Float32);
            }
            other => panic!("embedding should be FixedSizeList<Float32>, got {other:?}"),
        }
        assert!(!f.is_nullable(), "embedding must be non-nullable");
    }

    #[test]
    fn edge_schema_has_graph_columns() {
        let s = memory_edges_schema();
        for c in ["id", "src", "dst", "type", "realm", "target_namespace"] {
            assert!(s.field_with_name(c).is_ok(), "missing column {c}");
        }
    }

    #[test]
    fn node_schema_has_bitemporal_columns() {
        let s = memory_nodes_schema(384);
        for c in BITEMPORAL_COLUMNS {
            let f = s.field_with_name(c).unwrap_or_else(|_| panic!("missing column {c}"));
            assert!(f.is_nullable(), "{c} must be nullable for back-compat");
            assert_eq!(f.data_type(), &DataType::Utf8);
        }
    }

    #[test]
    fn node_schema_has_memory_class_columns() {
        let s = memory_nodes_schema(384);
        for c in MEMORY_CLASS_COLUMNS {
            let f = s.field_with_name(c).unwrap_or_else(|_| panic!("missing column {c}"));
            assert!(f.is_nullable(), "{c} must be nullable for back-compat");
            assert_eq!(f.data_type(), &DataType::Utf8);
        }
    }
}
