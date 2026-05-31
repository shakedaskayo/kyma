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
    ]))
}

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
}
