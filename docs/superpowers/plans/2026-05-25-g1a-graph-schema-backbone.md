# Graph Layer G1a — Schema-Graph Backbone Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a new `pensieve-graph` crate (wire types + `GraphProvider` trait + `SchemaGraphProvider`) and mount read-side `/v1/graph/*` HTTP endpoints in `pensieve-server`, so a client can fetch the catalog/schema as a property-graph (tables → nodes, inferred FK edges) in the exact JSON shape the web Context Graph UI will consume.

**Architecture:** `pensieve-graph` is dependency-light and decoupled from `pensieve-core`: it consumes a narrow `SchemaSource` trait (3 methods) rather than the full `Catalog`, so the provider is unit-testable with a hand-written fake. `pensieve-server` supplies a `CatalogSchemaSource` adapter over `Arc<dyn Catalog>` and exposes the provider through axum routes merged into the existing query `router()`. Only the synthetic `"schema"` graph exists in G1a — stored/registered graphs are G1b.

**Tech Stack:** Rust, `serde`/`serde_json`, `async-trait`, `thiserror`, `axum` (already used by `pensieve-server`), `tower::ServiceExt` for handler tests.

**Reference spec:** `docs/superpowers/specs/2026-05-25-graph-layer-context-graph-design.md` (§3.3 schema-graph, §5 GraphProvider + endpoints, §5.2 wire types).

---

## File structure

- Create `crates/pensieve-graph/Cargo.toml` — new workspace crate.
- Create `crates/pensieve-graph/src/lib.rs` — re-exports.
- Create `crates/pensieve-graph/src/types.rs` — wire types (`GraphNode`, `GraphRelationship`, `GraphStats`, `GraphPayload`, `EdgeExpansion`, `SearchHits`, `GraphSchema`, `GraphRef`, `Direction`, `Props`).
- Create `crates/pensieve-graph/src/source.rs` — `ColumnDef` + `SchemaSource` trait.
- Create `crates/pensieve-graph/src/provider.rs` — `GraphProvider` trait.
- Create `crates/pensieve-graph/src/schema_graph.rs` — `SchemaGraphProvider` + pure `infer_edges` helper.
- Modify `Cargo.toml` (workspace root) — add `crates/pensieve-graph` to members.
- Modify `crates/pensieve-server/Cargo.toml` — depend on `pensieve-graph`.
- Create `crates/pensieve-server/src/graph_handler.rs` — `CatalogSchemaSource` adapter + axum handlers + `graph_router`.
- Modify `crates/pensieve-server/src/lib.rs` — `pub mod graph_handler;` and `.merge(graph_handler::graph_router(state.clone()))` inside `router()`.

---

## Task 1: Create the `pensieve-graph` crate skeleton

**Files:**
- Create: `crates/pensieve-graph/Cargo.toml`
- Create: `crates/pensieve-graph/src/lib.rs`
- Modify: `Cargo.toml` (workspace root, `members` array)

- [ ] **Step 1: Add the crate to the workspace members**

In root `Cargo.toml`, add `"crates/pensieve-graph",` to the `members` array (place it right after `"crates/pensieve-kql",` to group with the engine crates).

- [ ] **Step 2: Write `crates/pensieve-graph/Cargo.toml`**

```toml
[package]
name = "pensieve-graph"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
authors.workspace = true

[dependencies]
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }
async-trait = { workspace = true }
anyhow = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt"] }
```

(Task 3 also references `anyhow`; it's included here from the start so the crate builds at each step.)

- [ ] **Step 3: Write `crates/pensieve-graph/src/lib.rs`**

```rust
//! First-class property-graph layer for pensieve: wire types, the `GraphProvider`
//! trait, and the synthetic schema-graph provider (catalog → property-graph).
//!
//! This crate is intentionally decoupled from `pensieve-core`: it consumes a
//! narrow [`SchemaSource`] trait rather than the full catalog, so providers
//! are unit-testable without a database.

pub mod provider;
pub mod schema_graph;
pub mod source;
pub mod types;

pub use provider::GraphProvider;
pub use schema_graph::SchemaGraphProvider;
pub use source::{ColumnDef, SchemaSource};
pub use types::*;
```

- [ ] **Step 4: Verify the workspace still builds**

Run: `cargo build -p pensieve-graph`
Expected: compiles (empty modules will fail — that's fixed in Task 2; if you run before Task 2 it errors on missing modules, which is fine). Skip running until Task 2 lands the modules.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/pensieve-graph/Cargo.toml crates/pensieve-graph/src/lib.rs
git commit -m "feat(graph): scaffold pensieve-graph crate"
```

---

## Task 2: Wire types (mirror the agentcy contract)

**Files:**
- Create: `crates/pensieve-graph/src/types.rs`
- Test: inline `#[cfg(test)]` in `types.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/pensieve-graph/src/types.rs`:

```rust
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
        // source_id is None -> omitted
        assert!(v["metadata"].get("source_id").is_none());
    }

    #[test]
    fn direction_serializes_lowercase() {
        assert_eq!(serde_json::to_value(Direction::Forward).unwrap(), "forward");
        assert_eq!(serde_json::to_value(Direction::Both).unwrap(), "both");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pensieve-graph types::tests`
Expected: FAIL — `GraphNode`/`NodeMetadata`/`Direction` not defined.

- [ ] **Step 3: Write the types (prepend above the test module)**

```rust
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
    /// `"schema"` (synthetic) or `"stored"` (registered — G1b).
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pensieve-graph types::tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-graph/src/types.rs
git commit -m "feat(graph): wire types mirroring the context-graph contract"
```

---

## Task 3: `SchemaSource` trait + `GraphProvider` trait

**Files:**
- Create: `crates/pensieve-graph/src/source.rs`
- Create: `crates/pensieve-graph/src/provider.rs`

- [ ] **Step 1: Write `crates/pensieve-graph/src/source.rs`**

```rust
//! Narrow read interface the schema-graph needs from a catalog. Keeping this
//! tiny (3 methods) lets `SchemaGraphProvider` be unit-tested with a fake.

use async_trait::async_trait;

/// A column as the schema-graph sees it (decoupled from `pensieve_core::ColumnInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    /// pensieve type token: `string`, `int`, `long`, `real`, `datetime`, `bool`, `dynamic`.
    pub type_: String,
    pub nullable: bool,
}

#[async_trait]
pub trait SchemaSource: Send + Sync {
    async fn databases(&self) -> anyhow::Result<Vec<String>>;
    async fn tables(&self, database: &str) -> anyhow::Result<Vec<String>>;
    async fn columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnDef>>;
}
```

- [ ] **Step 2: Confirm `anyhow` is present**

`anyhow = { workspace = true }` was already added to `crates/pensieve-graph/Cargo.toml` in Task 1. No change needed — just verify it's there before using `anyhow::Result` below.

- [ ] **Step 3: Write `crates/pensieve-graph/src/provider.rs`**

```rust
//! The provider abstraction every graph kind implements. G1a ships only
//! `SchemaGraphProvider`; G1b adds `StoredGraphProvider` behind the same trait.

use async_trait::async_trait;

use crate::types::{
    Direction, EdgeExpansion, GraphNode, GraphPayload, GraphSchema, GraphStats, SearchHits,
};

#[async_trait]
pub trait GraphProvider: Send + Sync {
    /// Capped sample of the whole graph: stats + nodes + edges.
    async fn overview(&self, realm: Option<&str>, limit: usize) -> anyhow::Result<GraphPayload>;
    /// A single node by id, or `None` if absent.
    async fn node(&self, id: &str) -> anyhow::Result<Option<GraphNode>>;
    /// Edges touching `ids` (respecting `dir`), plus the node ids newly reached.
    async fn neighbors(
        &self,
        ids: &[String],
        dir: Direction,
        only_internal: bool,
        limit: usize,
    ) -> anyhow::Result<EdgeExpansion>;
    /// BFS from `id` up to `depth` hops (both directions); returns the subgraph.
    async fn subgraph(&self, id: &str, depth: usize) -> anyhow::Result<GraphPayload>;
    /// Nodes whose name matches `text` (case-insensitive), optionally filtered
    /// by `labels` / `realm`.
    async fn search(
        &self,
        text: &str,
        labels: &[String],
        realm: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<SearchHits>;
    async fn stats(&self, realm: Option<&str>) -> anyhow::Result<GraphStats>;
    async fn schema(&self) -> anyhow::Result<GraphSchema>;
}
```

- [ ] **Step 4: Verify the crate builds**

Run: `cargo build -p pensieve-graph`
Expected: compiles (modules are referenced from `lib.rs`; `schema_graph` is still empty — add a temporary `pub mod schema_graph {}`? No — Task 4 creates it. To build now, comment out `pub mod schema_graph;` and its re-export in `lib.rs`, or proceed straight to Task 4 and build there.)

Proceed to Task 4 before building.

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-graph/Cargo.toml crates/pensieve-graph/src/source.rs crates/pensieve-graph/src/provider.rs
git commit -m "feat(graph): SchemaSource + GraphProvider traits"
```

---

## Task 4: Edge inference (pure helper, name-based FK heuristic)

**Files:**
- Create: `crates/pensieve-graph/src/schema_graph.rs`
- Test: inline `#[cfg(test)]` in `schema_graph.rs`

The schema-graph infers `REFERENCES` edges with a deterministic, value-free heuristic: a column named `<base>_id` in table A points to table B when B's name (lowercased) equals `<base>` or `<base>s`.

- [ ] **Step 1: Write the failing test**

Create `crates/pensieve-graph/src/schema_graph.rs` with only the test first:

```rust
#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::source::ColumnDef;

    fn col(name: &str) -> ColumnDef {
        ColumnDef { name: name.into(), type_: "string".into(), nullable: true }
    }

    #[test]
    fn infers_fk_edge_from_user_id_to_users() {
        let tables = vec![
            ("users".to_string(), vec![col("id"), col("email")]),
            ("orders".to_string(), vec![col("id"), col("user_id"), col("total")]),
        ];
        let edges = infer_edges("default", &tables);
        assert_eq!(edges.len(), 1);
        let e = &edges[0];
        assert_eq!(e.source_id, "default::orders");
        assert_eq!(e.target_id, "default::users");
        assert_eq!(e.relationship_type, "REFERENCES");
        assert_eq!(e.properties["via"], "user_id");
    }

    #[test]
    fn no_edge_when_no_matching_table() {
        let tables = vec![
            ("orders".to_string(), vec![col("id"), col("customer_id")]),
        ];
        assert!(infer_edges("default", &tables).is_empty());
    }

    #[test]
    fn plain_id_column_is_not_an_edge() {
        let tables = vec![("users".to_string(), vec![col("id")])];
        assert!(infer_edges("default", &tables).is_empty());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pensieve-graph schema_graph::edge_tests`
Expected: FAIL — `infer_edges` not defined.

- [ ] **Step 3: Implement `infer_edges` (prepend above the test module)**

```rust
//! The synthetic schema-graph: the catalog rendered as a property-graph.

use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::provider::GraphProvider;
use crate::source::{ColumnDef, SchemaSource};
use crate::types::{
    Direction, EdgeExpansion, GraphNode, GraphPayload, GraphRelationship, GraphSchema, GraphStats,
    NodeMetadata, Props, SearchHits,
};

/// Stable node id for a table.
fn table_node_id(database: &str, table: &str) -> String {
    format!("{database}::{table}")
}

/// Infer `REFERENCES` edges among tables of one database from `<base>_id`
/// column names. Pure + deterministic so it is trivially testable.
pub(crate) fn infer_edges(
    database: &str,
    tables: &[(String, Vec<ColumnDef>)],
) -> Vec<GraphRelationship> {
    let names: Vec<String> = tables.iter().map(|(n, _)| n.to_lowercase()).collect();
    let mut edges = Vec::new();
    for (tname, cols) in tables {
        for c in cols {
            let lname = c.name.to_lowercase();
            let Some(base) = lname.strip_suffix("_id") else { continue };
            if base.is_empty() {
                continue;
            }
            // Match a table whose name is `base` or `base` + "s".
            let target = names.iter().find(|n| *n == base || **n == format!("{base}s"));
            if let Some(target_lc) = target {
                // Resolve back to the original-cased table name.
                let target_name = tables
                    .iter()
                    .find(|(n, _)| n.to_lowercase() == *target_lc)
                    .map(|(n, _)| n.clone())
                    .unwrap();
                if target_name == *tname {
                    continue; // no self-edge from a column like `order_id` in `orders`
                }
                let mut props: Props = BTreeMap::new();
                props.insert("via".into(), serde_json::json!(c.name));
                edges.push(GraphRelationship {
                    id: format!(
                        "{}->{}:{}",
                        table_node_id(database, tname),
                        table_node_id(database, &target_name),
                        c.name
                    ),
                    source_id: table_node_id(database, tname),
                    target_id: table_node_id(database, &target_name),
                    relationship_type: "REFERENCES".into(),
                    properties: props,
                });
            }
        }
    }
    edges
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pensieve-graph schema_graph::edge_tests`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-graph/src/schema_graph.rs
git commit -m "feat(graph): name-based REFERENCES edge inference"
```

---

## Task 5: `SchemaGraphProvider` — build snapshot + node construction

**Files:**
- Modify: `crates/pensieve-graph/src/schema_graph.rs`
- Test: inline `#[cfg(test)]` in `schema_graph.rs` (a `FakeSource`)

- [ ] **Step 1: Write the failing test**

Append to `schema_graph.rs`:

```rust
#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::source::{ColumnDef, SchemaSource};

    struct FakeSource;

    fn col(name: &str, t: &str) -> ColumnDef {
        ColumnDef { name: name.into(), type_: t.into(), nullable: true }
    }

    #[async_trait]
    impl SchemaSource for FakeSource {
        async fn databases(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec!["default".into()])
        }
        async fn tables(&self, _db: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec!["users".into(), "orders".into()])
        }
        async fn columns(&self, _db: &str, table: &str) -> anyhow::Result<Vec<ColumnDef>> {
            Ok(match table {
                "users" => vec![col("id", "string"), col("email", "string")],
                "orders" => vec![col("id", "string"), col("user_id", "string")],
                _ => vec![],
            })
        }
    }

    fn provider() -> SchemaGraphProvider {
        SchemaGraphProvider::new(std::sync::Arc::new(FakeSource))
    }

    #[tokio::test]
    async fn overview_has_two_table_nodes_and_one_edge() {
        let p = provider();
        let payload = p.overview(None, 100).await.unwrap();
        assert_eq!(payload.nodes.len(), 2);
        assert!(payload.nodes.iter().all(|n| n.labels == vec!["Table".to_string()]));
        assert!(payload.nodes.iter().any(|n| n.id == "default::users"));
        assert_eq!(payload.edges.len(), 1);
        assert_eq!(payload.stats.total_nodes, 2);
        assert_eq!(payload.stats.total_relationships, 1);
        assert_eq!(payload.stats.label_counts["Table"], 2);
    }

    #[tokio::test]
    async fn node_lookup_returns_table_props() {
        let p = provider();
        let n = p.node("default::orders").await.unwrap().unwrap();
        assert_eq!(n.metadata.realm, "default");
        assert_eq!(n.properties["database"], "default");
        assert_eq!(n.properties["column_count"], 2);
        assert!(p.node("default::nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn search_filters_by_name_substring() {
        let p = provider();
        let hits = p.search("ord", &[], None, 10, 0).await.unwrap();
        assert_eq!(hits.total, 1);
        assert_eq!(hits.hits[0].id, "default::orders");
    }

    #[tokio::test]
    async fn neighbors_of_orders_returns_the_reference_edge() {
        let p = provider();
        let exp = p
            .neighbors(&["default::orders".into()], Direction::Both, true, 100)
            .await
            .unwrap();
        assert_eq!(exp.edges.len(), 1);
        assert_eq!(exp.new_node_ids, vec!["default::users".to_string()]);
    }

    #[tokio::test]
    async fn schema_reports_table_kind_and_references_edge() {
        let p = provider();
        let s = p.schema().await.unwrap();
        assert_eq!(s.node_kinds, vec!["Table".to_string()]);
        assert_eq!(s.edge_types, vec!["REFERENCES".to_string()]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pensieve-graph schema_graph::provider_tests`
Expected: FAIL — `SchemaGraphProvider` not defined.

- [ ] **Step 3: Implement `SchemaGraphProvider` (append to `schema_graph.rs`, before the test modules)**

```rust
use std::sync::Arc;

/// Synthetic graph computed live from a [`SchemaSource`]. Cheap to construct;
/// each call snapshots the catalog (the server already caches schema reads).
pub struct SchemaGraphProvider {
    source: Arc<dyn SchemaSource>,
}

impl SchemaGraphProvider {
    pub fn new(source: Arc<dyn SchemaSource>) -> Self {
        Self { source }
    }

    /// Build the full node + edge set, optionally restricted to one realm
    /// (= database). `now` timestamps are fixed per build for determinism.
    async fn build(&self, realm: Option<&str>) -> anyhow::Result<(Vec<GraphNode>, Vec<GraphRelationship>)> {
        let now = "1970-01-01T00:00:00Z".to_string(); // schema graph is timeless; stable value
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for db in self.source.databases().await? {
            if let Some(r) = realm {
                if r != db {
                    continue;
                }
            }
            let mut tables: Vec<(String, Vec<ColumnDef>)> = Vec::new();
            for t in self.source.tables(&db).await? {
                let cols = self.source.columns(&db, &t).await?;
                tables.push((t, cols));
            }
            for (tname, cols) in &tables {
                let mut props: Props = BTreeMap::new();
                props.insert("database".into(), serde_json::json!(db));
                props.insert("column_count".into(), serde_json::json!(cols.len()));
                props.insert(
                    "columns".into(),
                    serde_json::json!(cols
                        .iter()
                        .map(|c| serde_json::json!({"name": c.name, "type": c.type_, "nullable": c.nullable}))
                        .collect::<Vec<_>>()),
                );
                nodes.push(GraphNode {
                    id: table_node_id(&db, tname),
                    labels: vec!["Table".into()],
                    properties: props,
                    metadata: NodeMetadata {
                        created_at: now.clone(),
                        updated_at: now.clone(),
                        source_type: Some("schema".into()),
                        source_id: None,
                        realm: db.clone(),
                    },
                });
            }
            edges.extend(infer_edges(&db, &tables));
        }
        Ok((nodes, edges))
    }
}

fn compute_stats(nodes: &[GraphNode], edges: &[GraphRelationship]) -> GraphStats {
    let mut label_counts: BTreeMap<String, usize> = BTreeMap::new();
    for n in nodes {
        for l in &n.labels {
            *label_counts.entry(l.clone()).or_default() += 1;
        }
    }
    let mut relationship_type_counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in edges {
        *relationship_type_counts.entry(e.relationship_type.clone()).or_default() += 1;
    }
    GraphStats {
        total_nodes: nodes.len(),
        total_relationships: edges.len(),
        label_counts,
        relationship_type_counts,
    }
}

#[async_trait]
impl GraphProvider for SchemaGraphProvider {
    async fn overview(&self, realm: Option<&str>, limit: usize) -> anyhow::Result<GraphPayload> {
        let (mut nodes, edges) = self.build(realm).await?;
        let stats = compute_stats(&nodes, &edges);
        if nodes.len() > limit {
            nodes.truncate(limit);
        }
        let kept: std::collections::HashSet<&String> = nodes.iter().map(|n| &n.id).collect();
        let edges = edges
            .into_iter()
            .filter(|e| kept.contains(&e.source_id) && kept.contains(&e.target_id))
            .collect();
        Ok(GraphPayload { stats, nodes, edges })
    }

    async fn node(&self, id: &str) -> anyhow::Result<Option<GraphNode>> {
        let (nodes, _) = self.build(None).await?;
        Ok(nodes.into_iter().find(|n| n.id == id))
    }

    async fn neighbors(
        &self,
        ids: &[String],
        dir: Direction,
        _only_internal: bool,
        limit: usize,
    ) -> anyhow::Result<EdgeExpansion> {
        let (_, all_edges) = self.build(None).await?;
        let idset: std::collections::HashSet<&String> = ids.iter().collect();
        let mut edges = Vec::new();
        let mut new_ids = Vec::new();
        for e in all_edges {
            let touches = match dir {
                Direction::Forward => idset.contains(&e.source_id),
                Direction::Backward => idset.contains(&e.target_id),
                Direction::Both => idset.contains(&e.source_id) || idset.contains(&e.target_id),
            };
            if !touches {
                continue;
            }
            for end in [&e.source_id, &e.target_id] {
                if !idset.contains(end) && !new_ids.contains(end) {
                    new_ids.push(end.clone());
                }
            }
            edges.push(e);
            if edges.len() >= limit {
                break;
            }
        }
        Ok(EdgeExpansion { edges, new_node_ids: new_ids })
    }

    async fn subgraph(&self, id: &str, depth: usize) -> anyhow::Result<GraphPayload> {
        let (all_nodes, all_edges) = self.build(None).await?;
        let mut frontier = vec![id.to_string()];
        let mut visited: std::collections::HashSet<String> = frontier.iter().cloned().collect();
        let mut kept_edges: Vec<GraphRelationship> = Vec::new();
        for _ in 0..depth {
            let mut next = Vec::new();
            for e in &all_edges {
                let (a, b) = (&e.source_id, &e.target_id);
                let hit = frontier.contains(a) || frontier.contains(b);
                if hit && !kept_edges.iter().any(|k| k.id == e.id) {
                    kept_edges.push(e.clone());
                    for end in [a, b] {
                        if visited.insert(end.clone()) {
                            next.push(end.clone());
                        }
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        let nodes: Vec<GraphNode> = all_nodes.into_iter().filter(|n| visited.contains(&n.id)).collect();
        let stats = compute_stats(&nodes, &kept_edges);
        Ok(GraphPayload { stats, nodes, edges: kept_edges })
    }

    async fn search(
        &self,
        text: &str,
        labels: &[String],
        realm: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> anyhow::Result<SearchHits> {
        let (nodes, _) = self.build(realm).await?;
        let needle = text.to_lowercase();
        let mut matched: Vec<GraphNode> = nodes
            .into_iter()
            .filter(|n| {
                let name_ok = n.id.to_lowercase().contains(&needle);
                let label_ok = labels.is_empty() || labels.iter().any(|l| n.labels.contains(l));
                name_ok && label_ok
            })
            .collect();
        let total = matched.len();
        let hits = matched.drain(..).skip(offset).take(limit).collect();
        Ok(SearchHits { hits, total, limit, offset })
    }

    async fn stats(&self, realm: Option<&str>) -> anyhow::Result<GraphStats> {
        let (nodes, edges) = self.build(realm).await?;
        Ok(compute_stats(&nodes, &edges))
    }

    async fn schema(&self) -> anyhow::Result<GraphSchema> {
        let (nodes, edges) = self.build(None).await?;
        let mut edge_types: Vec<String> = edges.iter().map(|e| e.relationship_type.clone()).collect();
        edge_types.sort();
        edge_types.dedup();
        let mut property_keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if !nodes.is_empty() {
            property_keys.insert(
                "Table".into(),
                vec!["database".into(), "column_count".into(), "columns".into()],
            );
        }
        Ok(GraphSchema {
            node_kinds: if nodes.is_empty() { vec![] } else { vec!["Table".into()] },
            edge_types,
            property_keys,
        })
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pensieve-graph`
Expected: PASS (all `pensieve-graph` tests: types + edge + provider).

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-graph/src/schema_graph.rs
git commit -m "feat(graph): SchemaGraphProvider over SchemaSource"
```

---

## Task 6: `CatalogSchemaSource` adapter in pensieve-server

**Files:**
- Modify: `crates/pensieve-server/Cargo.toml` (add `pensieve-graph` dep)
- Create: `crates/pensieve-server/src/graph_handler.rs` (adapter only in this task)

- [ ] **Step 1: Add the dependency**

In `crates/pensieve-server/Cargo.toml` `[dependencies]`, add:
`pensieve-graph = { path = "../pensieve-graph" }`

- [ ] **Step 2: Write the failing test**

Create `crates/pensieve-server/src/graph_handler.rs`:

```rust
//! HTTP surface for the graph layer (`/v1/graph/*`). G1a serves only the
//! synthetic `"schema"` graph (catalog rendered as a property-graph).

use std::sync::Arc;

use async_trait::async_trait;
use pensieve_core::catalog::Catalog;
use pensieve_graph::{ColumnDef, SchemaSource};

/// Adapts the full [`Catalog`] down to the narrow [`SchemaSource`] the
/// schema-graph needs.
pub struct CatalogSchemaSource {
    catalog: Arc<dyn Catalog>,
}

impl CatalogSchemaSource {
    pub fn new(catalog: Arc<dyn Catalog>) -> Self {
        Self { catalog }
    }
}

#[async_trait]
impl SchemaSource for CatalogSchemaSource {
    async fn databases(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.catalog.list_databases().await?)
    }
    async fn tables(&self, database: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.catalog.list_tables(database).await?)
    }
    async fn columns(&self, database: &str, table: &str) -> anyhow::Result<Vec<ColumnDef>> {
        let cols = self.catalog.get_table_columns(database, table).await?;
        Ok(cols
            .into_iter()
            .map(|c| ColumnDef { name: c.name, type_: c.r#type, nullable: c.nullable })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pensieve_graph::{GraphProvider, SchemaGraphProvider};

    #[tokio::test]
    async fn adapter_feeds_schema_provider_from_seeded_catalog() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let source = Arc::new(CatalogSchemaSource::new(state.catalog.clone()));
        let provider = SchemaGraphProvider::new(source);
        let payload = provider.overview(None, 1000).await.unwrap();
        assert!(
            payload.nodes.iter().any(|n| n.id.ends_with("::otel_logs")),
            "expected an otel_logs table node, got: {:?}",
            payload.nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );
    }
}
```

- [ ] **Step 3: Register the module**

In `crates/pensieve-server/src/lib.rs`, add near the other `pub mod` lines (e.g. after `pub mod catalog_handler;`):

```rust
pub mod graph_handler;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p pensieve-server graph_handler::tests`
Expected: PASS (requires Docker for testcontainers Postgres, same as other `pensieve-server` handler tests).

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-server/Cargo.toml crates/pensieve-server/src/graph_handler.rs crates/pensieve-server/src/lib.rs
git commit -m "feat(graph): CatalogSchemaSource adapter in pensieve-server"
```

---

## Task 7: `/v1/graph/*` axum routes

**Files:**
- Modify: `crates/pensieve-server/src/graph_handler.rs` (add handlers + `graph_router`)
- Modify: `crates/pensieve-server/src/lib.rs` (merge `graph_router` into `router()`)

- [ ] **Step 1: Write the failing integration test**

Append to the `tests` module in `graph_handler.rs`:

```rust
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt; // for `oneshot`

    #[tokio::test]
    async fn overview_endpoint_returns_schema_graph_json() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/graph/schema/overview?limit=500")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["stats"]["total_nodes"].as_u64().unwrap() >= 1);
        assert!(v["nodes"].as_array().unwrap().iter().any(|n| n["id"]
            .as_str()
            .unwrap()
            .ends_with("::otel_logs")));
    }

    #[tokio::test]
    async fn unknown_graph_name_is_404() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(
                Request::builder()
                    .uri("/v1/graph/nope/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn list_endpoint_lists_schema_graph() {
        let state = crate::test_support::seeded_state_with_obs_otel_logs().await;
        let app = graph_router(state);
        let res = app
            .oneshot(Request::builder().uri("/v1/graph").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v[0]["name"], "schema");
        assert_eq!(v[0]["kind"], "schema");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pensieve-server graph_handler::tests::overview_endpoint_returns_schema_graph_json`
Expected: FAIL — `graph_router` not defined.

- [ ] **Step 3: Implement the handlers + router (append to `graph_handler.rs`, above the test module)**

```rust
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use pensieve_graph::{Direction, GraphProvider, GraphRef, SchemaGraphProvider};
use serde::Deserialize;

use crate::QueryState;

const SCHEMA_GRAPH: &str = "schema";

/// Build a `SchemaGraphProvider` over the request's catalog.
fn provider(state: &QueryState) -> SchemaGraphProvider {
    SchemaGraphProvider::new(Arc::new(CatalogSchemaSource::new(state.catalog.clone())))
}

/// Map any provider error to a 500 JSON envelope (consistent with other handlers).
fn err500(e: anyhow::Error) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": {"code": "graph", "message": e.to_string()}})),
    )
        .into_response()
}

/// 404 unless the graph name is the synthetic schema-graph (G1a).
fn ensure_schema(graph: &str) -> Result<(), Response> {
    if graph == SCHEMA_GRAPH {
        Ok(())
    } else {
        Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error": {"code": "not_found", "message": "unknown graph"}}))).into_response())
    }
}

#[derive(Deserialize)]
struct OverviewQuery {
    realm: Option<String>,
    #[serde(default = "default_overview_limit")]
    limit: usize,
}
fn default_overview_limit() -> usize {
    800
}

#[derive(Deserialize)]
struct RealmQuery {
    realm: Option<String>,
}

#[derive(Deserialize)]
struct SubgraphQuery {
    #[serde(default = "default_depth")]
    depth: usize,
}
fn default_depth() -> usize {
    2
}

#[derive(Deserialize)]
struct SearchBody {
    text: String,
    #[serde(default)]
    labels: Vec<String>,
    realm: Option<String>,
    #[serde(default = "default_search_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}
fn default_search_limit() -> usize {
    20
}

#[derive(Deserialize)]
struct NeighborsBody {
    node_ids: Vec<String>,
    #[serde(default = "default_direction")]
    direction: Direction,
    #[serde(default)]
    only_internal: bool,
    #[serde(default = "default_neighbors_limit")]
    limit: usize,
}
fn default_direction() -> Direction {
    Direction::Both
}
fn default_neighbors_limit() -> usize {
    200
}

async fn list_graphs(State(_state): State<QueryState>) -> Response {
    let refs = vec![GraphRef {
        name: SCHEMA_GRAPH.into(),
        kind: "schema".into(),
        description: "Catalog schema as a property-graph (tables + inferred references).".into(),
    }];
    (StatusCode::OK, Json(refs)).into_response()
}

async fn overview(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Query(q): Query<OverviewQuery>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).overview(q.realm.as_deref(), q.limit).await {
        Ok(p) => (StatusCode::OK, Json(p)).into_response(),
        Err(e) => err500(e),
    }
}

async fn stats(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Query(q): Query<RealmQuery>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).stats(q.realm.as_deref()).await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => err500(e),
    }
}

async fn schema(State(state): State<QueryState>, Path(graph): Path<String>) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).schema().await {
        Ok(s) => (StatusCode::OK, Json(s)).into_response(),
        Err(e) => err500(e),
    }
}

async fn node(
    State(state): State<QueryState>,
    Path((graph, id)): Path<(String, String)>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).node(&id).await {
        Ok(Some(n)) => (StatusCode::OK, Json(n)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": {"code": "not_found", "message": "no such node"}}))).into_response(),
        Err(e) => err500(e),
    }
}

async fn subgraph(
    State(state): State<QueryState>,
    Path((graph, id)): Path<(String, String)>,
    Query(q): Query<SubgraphQuery>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state).subgraph(&id, q.depth).await {
        Ok(p) => (StatusCode::OK, Json(p)).into_response(),
        Err(e) => err500(e),
    }
}

async fn search(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Json(body): Json<SearchBody>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state)
        .search(&body.text, &body.labels, body.realm.as_deref(), body.limit, body.offset)
        .await
    {
        Ok(h) => (StatusCode::OK, Json(h)).into_response(),
        Err(e) => err500(e),
    }
}

async fn neighbors(
    State(state): State<QueryState>,
    Path(graph): Path<String>,
    Json(body): Json<NeighborsBody>,
) -> Response {
    if let Err(r) = ensure_schema(&graph) {
        return r;
    }
    match provider(&state)
        .neighbors(&body.node_ids, body.direction, body.only_internal, body.limit)
        .await
    {
        Ok(x) => (StatusCode::OK, Json(x)).into_response(),
        Err(e) => err500(e),
    }
}

/// Read-only graph router. Caller wraps with the same `Role::Read` middleware
/// as the rest of the query surface.
pub fn graph_router(state: QueryState) -> Router {
    Router::new()
        .route("/v1/graph", get(list_graphs))
        .route("/v1/graph/:graph/overview", get(overview))
        .route("/v1/graph/:graph/stats", get(stats))
        .route("/v1/graph/:graph/schema", get(schema))
        .route("/v1/graph/:graph/nodes/:id", get(node))
        .route("/v1/graph/:graph/nodes/:id/subgraph", get(subgraph))
        .route("/v1/graph/:graph/search", post(search))
        .route("/v1/graph/:graph/neighbors", post(neighbors))
        .with_state(state)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pensieve-server graph_handler::tests`
Expected: PASS (adapter test + 3 endpoint tests).

- [ ] **Step 5: Mount the router in `router()`**

In `crates/pensieve-server/src/lib.rs`, inside `pub fn router(state: QueryState) -> Router`, add `.merge(graph_handler::graph_router(state.clone()))` to the builder chain (alongside `.merge(dash_read_router)`). The existing code already builds `dash_read_router` from a clone of `state`; mirror that — note `QueryState` is `Clone`, so `state.clone()` is valid. Ensure the final `.with_state(state)` still applies to the base router; the merged graph router carries its own state.

- [ ] **Step 6: Run the full server test suite for the graph + query surface**

Run: `cargo test -p pensieve-server graph_handler:: query`
Expected: PASS; no regression in existing query/router tests.

- [ ] **Step 7: Commit**

```bash
git add crates/pensieve-server/src/graph_handler.rs crates/pensieve-server/src/lib.rs
git commit -m "feat(graph): /v1/graph/* read endpoints over the schema-graph"
```

---

## Task 8: Manual smoke + plan close-out

**Files:** none (verification only)

- [ ] **Step 1: Build the whole workspace**

Run: `cargo build`
Expected: clean build including `pensieve-graph` + `pensieve-server`.

- [ ] **Step 2: Run the new tests once more, isolated**

Run: `cargo test -p pensieve-graph && cargo test -p pensieve-server graph_handler::`
Expected: all PASS.

- [ ] **Step 3: Record what G1b/G1c consume**

Confirm (by reading the test JSON) that the wire shape matches the spec §5.2 so the web SDK (G1c) and stored-graph provider (G1b) can rely on it: `overview` → `{stats, nodes[], edges[]}`; node `{id, labels[], properties{}, metadata{realm,…}}`; edge `{id, source_id, target_id, relationship_type, properties{}}`.

- [ ] **Step 4: Commit a short progress note (optional)**

No code change required. If desired, add a line to the spec's §9 marking G1a complete.

---

## Self-review notes (for the executor)

- **Spec coverage:** This plan implements spec §3.3 (schema-graph), §5.1 `GraphProvider` trait + `SchemaGraphProvider`, §5.2 wire types, and the read-side of §5.3 endpoints (`GET /v1/graph`, `overview`, `stats`, `schema`, `nodes/{id}`, `nodes/{id}/subgraph`, `POST search`, `POST neighbors`). **Out of scope here (later plans):** catalog graph registration + CLI + `StoredGraphProvider` (G1b); web Context Graph UI (G1c); run + seed script (G1d).
- **Edge inference is name-based** (`<base>_id` → `<base>`/`<base>s`), not value-based, per the G1a simplification. Spec §3.3's `find_references_to` value-based edges are a refinement tracked for a later pass; flagged in spec §10.
- **Type consistency:** `Direction` is `#[serde(rename_all = "lowercase")]` and used identically in `provider.rs`, `schema_graph.rs`, and `graph_handler.rs`. Node ids use the `"{db}::{table}"` form everywhere (`table_node_id`).
- **Docker note:** `pensieve-server` tests use testcontainers Postgres (via `test_support::seeded_state_*`), so they need Docker running — same precondition as the existing handler tests.
- **Auth:** `graph_router` is mounted inside the `Role::Read`-wrapped `router()`, so the endpoints inherit the same Bearer-token/`X-Database` posture as `/v1/query`. No per-handler auth added.
