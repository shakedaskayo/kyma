//! Minimal property-graph value model shared by generators, oracle, ingest
//! and harness.
//!
//! Deliberately tiny: nodes and edges are flat `Vec`s (stable, deterministic
//! order — generators must append in a reproducible order so the determinism
//! tests can compare whole graphs), properties are plain JSON maps.

use std::collections::HashMap;

/// A node in a [`TestGraph`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TestNode {
    /// Stable string id, unique within the graph (e.g. `"n42"`).
    pub id: String,
    /// Labels, most-significant first. The engine's node table stores a single
    /// `labels` string column compared with *equality* by Cypher `:Label`
    /// filters, so the ingest helper writes only the **first** label — see
    /// [`TestNode::primary_label`] and `crate::oracle::match_chain`.
    pub labels: Vec<String>,
    /// Arbitrary JSON properties (the ingest helper materializes `name`).
    pub props: serde_json::Map<String, serde_json::Value>,
}

impl TestNode {
    /// The label that actually lands in the engine's `labels` column (the
    /// first one), and the one label filters are evaluated against.
    pub fn primary_label(&self) -> Option<&str> {
        self.labels.first().map(String::as_str)
    }
}

/// A directed, typed edge in a [`TestGraph`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TestEdge {
    /// Source node id.
    pub src: String,
    /// Destination node id.
    pub dst: String,
    /// Relationship type (engine column literally named `type`).
    pub etype: String,
    /// Arbitrary JSON properties (not materialized by the ingest helper yet).
    pub props: serde_json::Map<String, serde_json::Value>,
}

/// A property graph small enough to hold in memory and replay against both
/// the oracle and a live engine.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TestGraph {
    pub nodes: Vec<TestNode>,
    pub edges: Vec<TestEdge>,
}

impl TestGraph {
    /// Empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Append a node with a single label and a `name` property equal to its id.
    pub fn add_node(&mut self, id: impl Into<String>, label: impl Into<String>) {
        let id = id.into();
        let mut props = serde_json::Map::new();
        props.insert("name".into(), serde_json::Value::String(id.clone()));
        self.nodes.push(TestNode {
            id,
            labels: vec![label.into()],
            props,
        });
    }

    /// Append a typed edge with no properties.
    pub fn add_edge(
        &mut self,
        src: impl Into<String>,
        dst: impl Into<String>,
        etype: impl Into<String>,
    ) {
        self.edges.push(TestEdge {
            src: src.into(),
            dst: dst.into(),
            etype: etype.into(),
            props: serde_json::Map::new(),
        });
    }

    /// `node id → index into self.nodes`.
    pub fn node_index(&self) -> HashMap<&str, usize> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect()
    }

    /// Outgoing adjacency: `src node id → indices into self.edges`.
    pub fn out_adjacency(&self) -> HashMap<&str, Vec<usize>> {
        let mut adj: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, e) in self.edges.iter().enumerate() {
            adj.entry(e.src.as_str()).or_default().push(i);
        }
        adj
    }

    /// Incoming adjacency: `dst node id → indices into self.edges`.
    pub fn in_adjacency(&self) -> HashMap<&str, Vec<usize>> {
        let mut adj: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, e) in self.edges.iter().enumerate() {
            adj.entry(e.dst.as_str()).or_default().push(i);
        }
        adj
    }

    /// Distinct edge types in first-appearance order.
    pub fn edge_types(&self) -> Vec<&str> {
        let mut seen = Vec::new();
        for e in &self.edges {
            if !seen.contains(&e.etype.as_str()) {
                seen.push(e.etype.as_str());
            }
        }
        seen
    }
}
