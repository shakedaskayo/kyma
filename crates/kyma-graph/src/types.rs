//! Wire types for the graph layer. These mirror the JSON contract the web
//! Context Graph UI consumes, so field names are load-bearing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Free-form node/edge property bag. `BTreeMap` keeps key order stable for
/// deterministic JSON in tests.
pub type Props = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeMetadata {
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub realm: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub labels: Vec<String>,
    #[serde(default)]
    pub properties: Props,
    pub metadata: NodeMetadata,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphRelationship {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relationship_type: String,
    #[serde(default)]
    pub properties: Props,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_nodes: usize,
    pub total_relationships: usize,
    pub label_counts: BTreeMap<String, usize>,
    pub relationship_type_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPayload {
    pub stats: GraphStats,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphRelationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeExpansion {
    pub edges: Vec<GraphRelationship>,
    pub new_node_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHits {
    pub hits: Vec<GraphNode>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSchema {
    pub node_kinds: Vec<String>,
    pub edge_types: Vec<String>,
    pub property_keys: BTreeMap<String, Vec<String>>,
}

/// One entry in the `GET /v1/graph` listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphRef {
    pub name: String,
    /// `"schema"` (synthetic) or `"stored"` (registered — later).
    pub kind: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Forward,
    Backward,
    Both,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_node_serializes_to_agentcy_shape() {
        let node = GraphNode {
            id: "default::otel_logs".into(),
            labels: vec!["Table".into()],
            properties: [("database".to_string(), serde_json::json!("default"))]
                .into_iter()
                .collect(),
            metadata: NodeMetadata {
                created_at: "2026-05-25T00:00:00Z".into(),
                updated_at: "2026-05-25T00:00:00Z".into(),
                source_type: Some("schema".into()),
                source_id: None,
                realm: "default".into(),
            },
        };
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["id"], "default::otel_logs");
        assert_eq!(v["labels"][0], "Table");
        assert_eq!(v["properties"]["database"], "default");
        assert_eq!(v["metadata"]["realm"], "default");
        assert_eq!(v["metadata"]["source_type"], "schema");
        assert!(v["metadata"].get("source_id").is_none());
    }

    #[test]
    fn direction_serializes_lowercase() {
        assert_eq!(serde_json::to_value(Direction::Forward).unwrap(), "forward");
        assert_eq!(serde_json::to_value(Direction::Both).unwrap(), "both");
    }
}
