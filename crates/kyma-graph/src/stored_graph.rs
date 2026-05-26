//! Stored-graph provider: serves a registered property-graph by querying its
//! node/edge tables via a `GraphQueryExecutor` and shaping rows into wire types.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::executor::{GraphQueryExecutor, JsonRow, StoredGraphConfig};
use crate::provider::GraphProvider;
use crate::types::{
    Direction, EdgeExpansion, GraphNode, GraphPayload, GraphRelationship, GraphSchema, GraphStats,
    NodeMetadata, Props, SearchHits,
};

const NOW: &str = "1970-01-01T00:00:00Z";

fn ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
fn lit(v: &str) -> String {
    format!("'{}'", v.replace('\'', "''"))
}
fn in_list(values: &[String]) -> String {
    values.iter().map(|v| lit(v)).collect::<Vec<_>>().join(",")
}
/// JSON value → plain string (unwrap JSON strings; stringify the rest).
fn as_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A deduplicated node source: one row per node id.
///
/// Connector tables are append-only, so a *continuous* connector re-ingests a
/// fresh row for every node on each poll (metadata is re-emitted even when
/// unchanged). Without dedup, the graph would render N copies of every node
/// after N polls. We keep one row per id via a `ROW_NUMBER()` window. There is
/// no reliable ingest-order column to pick the truly-latest row, so the choice
/// is deterministic-arbitrary; re-ingested rows are identical in practice.
fn node_source(c: &StoredGraphConfig) -> String {
    format!(
        "(select * from (select *, row_number() over (partition by {id} order by {id}) as __rn from {t}) where __rn = 1)",
        id = ident(&c.id_col), t = ident(&c.node_table),
    )
}
/// A deduplicated edge source: one row per (src, dst, type), matching the
/// connector's deterministic edge identity (`hash(src, type, dst)`).
fn edge_source(c: &StoredGraphConfig) -> String {
    format!(
        "(select * from (select *, row_number() over (partition by {s},{d},{ty} order by {s}) as __rn from {t}) where __rn = 1)",
        s = ident(&c.src_col), d = ident(&c.dst_col), ty = ident(&c.type_col), t = ident(&c.edge_table),
    )
}

pub(crate) fn node_sample_sql(c: &StoredGraphConfig, limit: usize) -> String {
    format!("select * from {} limit {}", node_source(c), limit)
}
pub(crate) fn edge_sample_sql(c: &StoredGraphConfig, limit: usize) -> String {
    format!("select * from {} limit {}", edge_source(c), limit)
}
pub(crate) fn node_by_id_sql(c: &StoredGraphConfig, id: &str) -> String {
    format!("select * from {} where {} = {} limit 1", node_source(c), ident(&c.id_col), lit(id))
}
pub(crate) fn neighbors_sql(c: &StoredGraphConfig, ids: &[String], dir: Direction, limit: usize) -> String {
    let list = in_list(ids);
    let pred = match dir {
        Direction::Forward => format!("{} in ({list})", ident(&c.src_col)),
        Direction::Backward => format!("{} in ({list})", ident(&c.dst_col)),
        Direction::Both => format!("{} in ({list}) or {} in ({list})", ident(&c.src_col), ident(&c.dst_col)),
    };
    format!("select * from {} where {pred} limit {limit}", edge_source(c))
}
pub(crate) fn search_sql(c: &StoredGraphConfig, text: &str, limit: usize, offset: usize) -> String {
    let needle = lit(&format!("%{}%", text.to_lowercase()));
    format!(
        "select * from {t} where lower(cast({id} as varchar)) like {n} or lower(cast({lbl} as varchar)) like {n} limit {limit} offset {offset}",
        t = node_source(c), id = ident(&c.id_col), lbl = ident(&c.label_col), n = needle,
    )
}
/// Count rows of an already-built (possibly deduplicated) source expression.
pub(crate) fn count_sql(source: &str) -> String {
    format!("select count(*) as n from {}", source)
}
/// Group-count over an already-built source expression.
pub(crate) fn group_count_sql(source: &str, col: &str) -> String {
    format!("select cast({c} as varchar) as k, count(*) as n from {t} group by {c}", c = ident(col), t = source)
}

fn parse_labels(v: Option<&serde_json::Value>) -> Vec<String> {
    match v {
        Some(serde_json::Value::Array(a)) => a.iter().map(as_str).collect(),
        Some(serde_json::Value::String(s)) => vec![s.clone()],
        Some(other) => vec![other.to_string()],
        None => vec![],
    }
}

pub(crate) fn row_to_node(c: &StoredGraphConfig, row: &JsonRow) -> GraphNode {
    let id = row.get(&c.id_col).map(as_str).unwrap_or_default();
    let labels = parse_labels(row.get(&c.label_col));
    let realm = c.realm_col.as_ref().and_then(|rc| row.get(rc)).map(as_str).unwrap_or_else(|| c.database.clone());
    let role_cols: [&str; 3] = [c.id_col.as_str(), c.label_col.as_str(), c.realm_col.as_deref().unwrap_or("")];
    let mut props: Props = BTreeMap::new();
    for (k, v) in row {
        if role_cols.contains(&k.as_str()) || k == "__rn" { continue; }
        props.insert(k.clone(), v.clone());
    }
    GraphNode {
        id, labels, properties: props,
        metadata: NodeMetadata { created_at: NOW.into(), updated_at: NOW.into(), source_type: Some("stored".into()), source_id: None, realm },
    }
}

pub(crate) fn row_to_edge(c: &StoredGraphConfig, row: &JsonRow) -> GraphRelationship {
    let src = row.get(&c.src_col).map(as_str).unwrap_or_default();
    let dst = row.get(&c.dst_col).map(as_str).unwrap_or_default();
    let ty = row.get(&c.type_col).map(as_str).unwrap_or_default();
    let role_cols = [c.src_col.as_str(), c.dst_col.as_str(), c.type_col.as_str()];
    let mut props: Props = BTreeMap::new();
    for (k, v) in row {
        if role_cols.contains(&k.as_str()) || k == "__rn" { continue; }
        props.insert(k.clone(), v.clone());
    }
    GraphRelationship {
        id: format!("{src}->{dst}:{ty}"),
        source_id: src, target_id: dst, relationship_type: ty, properties: props,
    }
}

pub struct StoredGraphProvider {
    cfg: StoredGraphConfig,
    exec: Arc<dyn GraphQueryExecutor>,
}

impl StoredGraphProvider {
    pub fn new(cfg: StoredGraphConfig, exec: Arc<dyn GraphQueryExecutor>) -> Self {
        Self { cfg, exec }
    }
    async fn rows(&self, sql: String) -> anyhow::Result<Vec<JsonRow>> {
        self.exec.query(&self.cfg.database, sql).await
    }
    fn count_of(rows: &[JsonRow]) -> usize {
        rows.first().and_then(|r| r.get("n")).and_then(|v| v.as_u64()).unwrap_or(0) as usize
    }
}

async fn stats_for(p: &StoredGraphProvider) -> anyhow::Result<GraphStats> {
    let total_nodes = StoredGraphProvider::count_of(&p.rows(count_sql(&node_source(&p.cfg))).await?);
    let total_relationships = StoredGraphProvider::count_of(&p.rows(count_sql(&edge_source(&p.cfg))).await?);
    let mut label_counts = BTreeMap::new();
    for r in p.rows(group_count_sql(&node_source(&p.cfg), &p.cfg.label_col)).await? {
        let k = r.get("k").map(as_str).unwrap_or_default();
        let n = r.get("n").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        label_counts.insert(k, n);
    }
    let mut relationship_type_counts = BTreeMap::new();
    for r in p.rows(group_count_sql(&edge_source(&p.cfg), &p.cfg.type_col)).await? {
        let k = r.get("k").map(as_str).unwrap_or_default();
        let n = r.get("n").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        relationship_type_counts.insert(k, n);
    }
    Ok(GraphStats { total_nodes, total_relationships, label_counts, relationship_type_counts })
}

#[async_trait]
impl GraphProvider for StoredGraphProvider {
    async fn overview(&self, _realm: Option<&str>, limit: usize) -> anyhow::Result<GraphPayload> {
        let nodes: Vec<GraphNode> = self.rows(node_sample_sql(&self.cfg, limit)).await?
            .iter().map(|r| row_to_node(&self.cfg, r)).collect();
        let kept: std::collections::HashSet<&String> = nodes.iter().map(|n| &n.id).collect();
        let edges: Vec<GraphRelationship> = self.rows(edge_sample_sql(&self.cfg, limit.saturating_mul(4))).await?
            .iter().map(|r| row_to_edge(&self.cfg, r))
            .filter(|e| kept.contains(&e.source_id) && kept.contains(&e.target_id))
            .collect();
        let stats = stats_for(self).await?;
        Ok(GraphPayload { stats, nodes, edges })
    }
    async fn node(&self, id: &str) -> anyhow::Result<Option<GraphNode>> {
        Ok(self.rows(node_by_id_sql(&self.cfg, id)).await?.first().map(|r| row_to_node(&self.cfg, r)))
    }
    async fn neighbors(&self, ids: &[String], dir: Direction, _only_internal: bool, limit: usize) -> anyhow::Result<EdgeExpansion> {
        if ids.is_empty() { return Ok(EdgeExpansion { edges: vec![], new_node_ids: vec![] }); }
        let edges: Vec<GraphRelationship> = self.rows(neighbors_sql(&self.cfg, ids, dir, limit)).await?
            .iter().map(|r| row_to_edge(&self.cfg, r)).collect();
        let idset: std::collections::HashSet<&String> = ids.iter().collect();
        let mut new_ids = Vec::new();
        for e in &edges {
            for end in [&e.source_id, &e.target_id] {
                if !idset.contains(end) && !new_ids.contains(end) { new_ids.push(end.clone()); }
            }
        }
        Ok(EdgeExpansion { edges, new_node_ids: new_ids })
    }
    async fn subgraph(&self, id: &str, depth: usize) -> anyhow::Result<GraphPayload> {
        let mut frontier = vec![id.to_string()];
        let mut all_node_ids: std::collections::HashSet<String> = frontier.iter().cloned().collect();
        let mut edges: Vec<GraphRelationship> = Vec::new();
        for _ in 0..depth.max(1) {
            if frontier.is_empty() { break; }
            let exp = self.neighbors(&frontier, Direction::Both, true, 500).await?;
            let mut next = Vec::new();
            for e in exp.edges {
                if !edges.iter().any(|k| k.id == e.id) { edges.push(e); }
            }
            for nid in exp.new_node_ids {
                if all_node_ids.insert(nid.clone()) { next.push(nid); }
            }
            frontier = next;
        }
        // fetch node objects for the collected ids
        let mut nodes = Vec::new();
        for nid in &all_node_ids {
            if let Some(n) = self.node(nid).await? { nodes.push(n); }
        }
        let stats = GraphStats {
            total_nodes: nodes.len(), total_relationships: edges.len(),
            label_counts: BTreeMap::new(), relationship_type_counts: BTreeMap::new(),
        };
        Ok(GraphPayload { stats, nodes, edges })
    }
    async fn search(&self, text: &str, labels: &[String], _realm: Option<&str>, limit: usize, offset: usize) -> anyhow::Result<SearchHits> {
        let mut hits: Vec<GraphNode> = self.rows(search_sql(&self.cfg, text, limit, offset)).await?
            .iter().map(|r| row_to_node(&self.cfg, r)).collect();
        if !labels.is_empty() {
            hits.retain(|n| labels.iter().any(|l| n.labels.contains(l)));
        }
        let total = hits.len();
        Ok(SearchHits { hits, total, limit, offset })
    }
    async fn stats(&self, _realm: Option<&str>) -> anyhow::Result<GraphStats> {
        stats_for(self).await
    }
    async fn schema(&self) -> anyhow::Result<GraphSchema> {
        let stats = stats_for(self).await?;
        Ok(GraphSchema {
            node_kinds: stats.label_counts.keys().cloned().collect(),
            edge_types: stats.relationship_type_counts.keys().cloned().collect(),
            property_keys: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod sql_tests {
    use super::*;
    use crate::executor::StoredGraphConfig;

    fn cfg() -> StoredGraphConfig {
        StoredGraphConfig {
            database: "kg".into(), node_table: "kg_nodes".into(), edge_table: "kg_edges".into(),
            id_col: "id".into(), label_col: "labels".into(),
            src_col: "src".into(), dst_col: "dst".into(), type_col: "type".into(),
            realm_col: Some("realm".into()),
        }
    }

    #[test]
    fn node_by_id_sql_quotes_and_escapes() {
        let s = node_by_id_sql(&cfg(), "a'b");
        assert!(s.contains(r#"from "kg_nodes""#), "{s}");
        assert!(s.contains(r#""id" = 'a''b'"#), "{s}");
        assert!(s.to_lowercase().contains("limit 1"));
    }

    #[test]
    fn neighbors_sql_both_directions() {
        let s = neighbors_sql(&cfg(), &["x".into(), "y".into()], Direction::Both, 50);
        assert!(s.contains(r#""src" in ('x','y')"#), "{s}");
        assert!(s.contains(r#""dst" in ('x','y')"#), "{s}");
        assert!(s.to_lowercase().contains("limit 50"));
    }

    #[test]
    fn node_sample_dedups_by_id() {
        // Append-only connector tables accumulate a fresh row per node on every
        // poll; the node source must collapse to one row per id.
        let s = node_sample_sql(&cfg(), 50).to_lowercase();
        assert!(s.contains("row_number()"), "{s}");
        assert!(s.contains(r#"partition by "id""#), "{s}");
        assert!(s.contains("kg_nodes"), "{s}");
        assert!(s.contains("limit 50"), "{s}");
    }

    #[test]
    fn edge_sample_dedups_by_src_dst_type() {
        let s = edge_sample_sql(&cfg(), 50).to_lowercase();
        assert!(s.contains("row_number()"), "{s}");
        assert!(s.contains(r#"partition by "src","dst","type""#), "{s}");
        assert!(s.contains("kg_edges"), "{s}");
    }

    #[test]
    fn count_and_group_count_run_over_deduped_source() {
        // stats must count distinct ids, not raw rows.
        let cnt = count_sql(&node_source(&cfg())).to_lowercase();
        assert!(cnt.contains("count(*)") && cnt.contains("row_number()"), "{cnt}");
        let grp = group_count_sql(&node_source(&cfg()), &cfg().label_col).to_lowercase();
        assert!(grp.contains("group by") && grp.contains("row_number()"), "{grp}");
    }

    #[test]
    fn row_to_node_uses_roles() {
        let mut row = JsonRow::new();
        row.insert("id".into(), serde_json::json!("n1"));
        row.insert("labels".into(), serde_json::json!("Service"));
        row.insert("realm".into(), serde_json::json!("prod"));
        row.insert("owner".into(), serde_json::json!("team-a"));
        let n = row_to_node(&cfg(), &row);
        assert_eq!(n.id, "n1");
        assert_eq!(n.labels, vec!["Service".to_string()]);
        assert_eq!(n.metadata.realm, "prod");
        assert_eq!(n.properties.get("owner").unwrap(), &serde_json::json!("team-a"));
        assert!(!n.properties.contains_key("id")); // role columns excluded from props
    }

    #[test]
    fn row_to_edge_uses_roles() {
        let mut row = JsonRow::new();
        row.insert("src".into(), serde_json::json!("a"));
        row.insert("dst".into(), serde_json::json!("b"));
        row.insert("type".into(), serde_json::json!("CALLS"));
        row.insert("weight".into(), serde_json::json!(5));
        let e = row_to_edge(&cfg(), &row);
        assert_eq!(e.source_id, "a");
        assert_eq!(e.target_id, "b");
        assert_eq!(e.relationship_type, "CALLS");
        assert_eq!(e.properties.get("weight").unwrap(), &serde_json::json!(5));
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    use crate::executor::{GraphQueryExecutor, JsonRow, StoredGraphConfig};

    struct FakeExec;
    fn row(pairs: &[(&str, serde_json::Value)]) -> JsonRow {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
    #[async_trait]
    impl GraphQueryExecutor for FakeExec {
        async fn query(&self, _db: &str, sql: String) -> anyhow::Result<Vec<JsonRow>> {
            let s = sql.to_lowercase();
            if s.contains("count(*)") && s.contains("group by") {
                return Ok(vec![row(&[("k", serde_json::json!("Service")), ("n", serde_json::json!(2))])]);
            }
            if s.contains("count(*)") {
                return Ok(vec![row(&[("n", serde_json::json!(2))])]);
            }
            if s.contains("kg_nodes") {
                return Ok(vec![
                    row(&[("id", serde_json::json!("a")), ("labels", serde_json::json!("Service"))]),
                    row(&[("id", serde_json::json!("b")), ("labels", serde_json::json!("Service"))]),
                ]);
            }
            if s.contains("kg_edges") {
                return Ok(vec![row(&[("src", serde_json::json!("a")), ("dst", serde_json::json!("b")), ("type", serde_json::json!("CALLS"))])]);
            }
            Ok(vec![])
        }
    }
    fn provider() -> StoredGraphProvider {
        StoredGraphProvider::new(
            StoredGraphConfig { database: "kg".into(), node_table: "kg_nodes".into(), edge_table: "kg_edges".into(),
                id_col: "id".into(), label_col: "labels".into(), src_col: "src".into(), dst_col: "dst".into(), type_col: "type".into(), realm_col: None },
            std::sync::Arc::new(FakeExec),
        )
    }

    #[tokio::test]
    async fn overview_shapes_nodes_edges_and_stats() {
        let p = provider();
        let ov = p.overview(None, 100).await.unwrap();
        assert_eq!(ov.nodes.len(), 2);
        assert_eq!(ov.edges.len(), 1);
        assert_eq!(ov.edges[0].relationship_type, "CALLS");
        assert_eq!(ov.stats.total_nodes, 2);
        assert_eq!(ov.stats.label_counts.get("Service").copied(), Some(2));
    }
    #[tokio::test]
    async fn neighbors_collects_new_ids() {
        let p = provider();
        let exp = p.neighbors(&["a".into()], Direction::Both, true, 50).await.unwrap();
        assert_eq!(exp.edges.len(), 1);
        assert_eq!(exp.new_node_ids, vec!["b".to_string()]);
    }
}
