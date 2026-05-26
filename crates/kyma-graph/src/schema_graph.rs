//! The synthetic schema-graph: the catalog rendered as a property-graph.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::provider::GraphProvider;
use crate::source::{ColumnDef, SchemaSource};
use crate::types::{
    Direction, EdgeExpansion, GraphNode, GraphPayload, GraphRelationship, GraphSchema, GraphStats,
    NodeMetadata, Props, SearchHits,
};

// The schema graph is timeless; use a fixed stable timestamp for deterministic JSON.
const SCHEMA_TS: &str = "1970-01-01T00:00:00Z";

/// True if `name` is a `<g>_nodes`/`<g>_edges` table that backs a registered
/// property-graph (its sibling table exists). These are storage plumbing for
/// the connector/stored graphs and are hidden from the catalog *schema* graph —
/// surfacing them as standalone `Table` nodes is confusing.
fn is_graph_storage_table(name: &str, all: &std::collections::HashSet<String>) -> bool {
    if let Some(base) = name.strip_suffix("_nodes") {
        return all.contains(&format!("{base}_edges"));
    }
    if let Some(base) = name.strip_suffix("_edges") {
        return all.contains(&format!("{base}_nodes"));
    }
    false
}

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
                let target_name = tables
                    .iter()
                    .find(|(n, _)| n.to_lowercase() == *target_lc)
                    .map(|(n, _)| n.clone())
                    .unwrap();
                if target_name == *tname {
                    continue; // no self-edge
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
    /// (= database). Timestamps are a fixed stable value (the schema graph is
    /// timeless), keeping JSON deterministic.
    async fn build(
        &self,
        realm: Option<&str>,
    ) -> anyhow::Result<(Vec<GraphNode>, Vec<GraphRelationship>)> {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for db in self.source.databases().await? {
            if let Some(r) = realm {
                if r != db {
                    continue;
                }
            }
            let all_names: std::collections::HashSet<String> =
                self.source.tables(&db).await?.into_iter().collect();
            let mut tables: Vec<(String, Vec<ColumnDef>)> = Vec::new();
            for t in &all_names {
                // Hide graph-storage plumbing (`<g>_nodes`/`<g>_edges` pairs).
                if is_graph_storage_table(t, &all_names) {
                    continue;
                }
                let cols = self.source.columns(&db, t).await?;
                tables.push((t.clone(), cols));
            }
            tables.sort_by(|a, b| a.0.cmp(&b.0));
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
                        created_at: SCHEMA_TS.into(),
                        updated_at: SCHEMA_TS.into(),
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
        // stats reflect the FULL catalog; nodes/edges below are the capped view, so stats.total_* may exceed the returned slice lengths.
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
        // `only_internal` is N/A for the schema graph: every node is internal by definition.
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
        let nodes: Vec<GraphNode> =
            all_nodes.into_iter().filter(|n| visited.contains(&n.id)).collect();
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
                let table_name = n.id.rsplit("::").next().unwrap_or(n.id.as_str());
                let name_ok = table_name.to_lowercase().contains(&needle);
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
        let mut edge_types: Vec<String> =
            edges.iter().map(|e| e.relationship_type.clone()).collect();
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

#[cfg(test)]
mod edge_tests {
    use super::*;
    use crate::source::ColumnDef;

    fn col(name: &str) -> ColumnDef {
        ColumnDef { name: name.into(), type_: "string".into(), nullable: true }
    }

    #[test]
    fn hides_graph_storage_table_pairs() {
        let all: std::collections::HashSet<String> = [
            "github_nodes", "github_edges", "kg_nodes", "kg_edges", "api_calls", "users", "lonely_nodes",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // `<g>_nodes`/`<g>_edges` pairs that back a graph are hidden.
        assert!(is_graph_storage_table("github_nodes", &all));
        assert!(is_graph_storage_table("github_edges", &all));
        assert!(is_graph_storage_table("kg_nodes", &all));
        assert!(is_graph_storage_table("kg_edges", &all));
        // Real schema tables stay visible.
        assert!(!is_graph_storage_table("api_calls", &all));
        assert!(!is_graph_storage_table("users", &all));
        // A lone `_nodes` table with no `_edges` sibling is NOT hidden.
        assert!(!is_graph_storage_table("lonely_nodes", &all));
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
    async fn search_does_not_match_database_prefix() {
        let p = provider();
        // "default" is the database name (part of every id) but not a table name.
        let hits = p.search("default", &[], None, 10, 0).await.unwrap();
        assert_eq!(hits.total, 0, "search must match table names, not the db prefix");
        // sanity: a real table-name substring still matches
        let hits2 = p.search("ord", &[], None, 10, 0).await.unwrap();
        assert_eq!(hits2.total, 1);
    }

    #[tokio::test]
    async fn schema_reports_table_kind_and_references_edge() {
        let p = provider();
        let s = p.schema().await.unwrap();
        assert_eq!(s.node_kinds, vec!["Table".to_string()]);
        assert_eq!(s.edge_types, vec!["REFERENCES".to_string()]);
    }

    #[tokio::test]
    async fn overview_caps_nodes_but_stats_reflect_full_graph() {
        let p = provider();
        let payload = p.overview(None, 1).await.unwrap();
        assert_eq!(payload.nodes.len(), 1, "nodes capped to limit");
        assert_eq!(payload.stats.total_nodes, 2, "stats reflect full graph");
        // the single FK edge needs both endpoints; with only 1 node kept it's filtered out
        assert_eq!(payload.edges.len(), 0);
        assert_eq!(payload.stats.total_relationships, 1);
    }

    struct ChainSource;

    #[async_trait]
    impl SchemaSource for ChainSource {
        async fn databases(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec!["default".into()])
        }
        async fn tables(&self, _db: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec!["as_".into(), "bs".into(), "cs".into()])
        }
        async fn columns(&self, _db: &str, table: &str) -> anyhow::Result<Vec<ColumnDef>> {
            // as_ -> bs (via b_id), bs -> cs (via c_id)
            Ok(match table {
                "as_" => vec![col("id", "string"), col("b_id", "string")],
                "bs" => vec![col("id", "string"), col("c_id", "string")],
                "cs" => vec![col("id", "string")],
                _ => vec![],
            })
        }
    }

    #[tokio::test]
    async fn subgraph_two_hops_collects_chain() {
        let p = SchemaGraphProvider::new(std::sync::Arc::new(ChainSource));
        // depth 2 from as_ should reach bs (hop 1) and cs (hop 2)
        let sg = p.subgraph("default::as_", 2).await.unwrap();
        let ids: std::collections::HashSet<String> = sg.nodes.iter().map(|n| n.id.clone()).collect();
        assert!(ids.contains("default::as_"));
        assert!(ids.contains("default::bs"));
        assert!(ids.contains("default::cs"));
        assert_eq!(sg.edges.len(), 2);
    }
}
