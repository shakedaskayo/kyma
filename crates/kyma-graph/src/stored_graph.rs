//! Stored-graph provider: serves a registered property-graph by querying its
//! node/edge tables via a `GraphQueryExecutor` and shaping rows into wire types.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use kyma_graph_topo::{CsrGraph, Direction as TopoDirection};

use crate::executor::{GraphQueryExecutor, JsonRow, StoredGraphConfig};
use crate::provider::GraphProvider;
use crate::types::{
    Direction, EdgeExpansion, GraphNode, GraphPayload, GraphRelationship, GraphSchema, GraphStats,
    NodeMetadata, Props, SearchHits,
};

const NOW: &str = "1970-01-01T00:00:00Z";

/// Edge ceiling for the in-memory CSR subgraph path. When a graph's deduped
/// edge set fits under this, `subgraph` builds the whole topology once and does
/// BFS in RAM (one scan, any depth); above it, the per-hop SQL path is kept so
/// large graphs never attempt a full-topology load. `KYMA_GRAPH_TOPO_MAX_EDGES`
/// overrides the default.
fn topo_max_edges() -> usize {
    std::env::var("KYMA_GRAPH_TOPO_MAX_EDGES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(200_000)
}

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
/// DataSource tables are append-only, so a *continuous* data source re-ingests a
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
/// data source's deterministic edge identity (`hash(src, type, dst)`).
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

/// The conventional dynamic-JSON catch-all column (see kyma's default table
/// schema). Data sources stash entity-specific fields here as a JSON blob.
const PROPS_COL: &str = "props";

/// Decode an even-length lowercase/uppercase hex string into bytes.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || s.len() % 2 != 0 {
        return None;
    }
    let val = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        out.push((val(b[i])? << 4) | val(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

/// Decode the `props` catch-all blob into an object so its fields surface as
/// real properties. `ensure_table` types `props` as Binary, so the executor
/// (arrow-json) renders it as a hex string; we hex-decode → UTF-8 → JSON. Also
/// handles a Utf8 `props` column whose value is the JSON text directly.
fn decode_props_blob(v: &serde_json::Value) -> Option<serde_json::Map<String, serde_json::Value>> {
    let s = v.as_str()?;
    if let Some(bytes) = hex_decode(s) {
        if let Ok(txt) = std::str::from_utf8(&bytes) {
            if let Ok(serde_json::Value::Object(m)) = serde_json::from_str(txt) {
                return Some(m);
            }
        }
    }
    if let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(s) {
        return Some(m);
    }
    None
}

/// Build the property map for a row: explicit (promoted) columns win; the
/// `props` blob is decoded and merged in to fill any remaining keys. Role
/// columns and the `__rn` dedup helper are excluded.
fn collect_props(row: &JsonRow, role_cols: &[&str]) -> Props {
    let mut props: Props = BTreeMap::new();
    let mut blob: Option<&serde_json::Value> = None;
    for (k, v) in row {
        if role_cols.contains(&k.as_str()) || k == "__rn" {
            continue;
        }
        if k == PROPS_COL {
            blob = Some(v);
            continue;
        }
        props.insert(k.clone(), v.clone());
    }
    if let Some(v) = blob {
        match decode_props_blob(v) {
            Some(obj) => {
                for (pk, pv) in obj {
                    props.entry(pk).or_insert(pv);
                }
            }
            // Undecodable (not JSON): keep the raw value so nothing is lost.
            None => {
                props.insert(PROPS_COL.to_string(), v.clone());
            }
        }
    }
    props
}

pub(crate) fn row_to_node(c: &StoredGraphConfig, row: &JsonRow) -> GraphNode {
    let id = row.get(&c.id_col).map(as_str).unwrap_or_default();
    let labels = parse_labels(row.get(&c.label_col));
    let realm = c.realm_col.as_ref().and_then(|rc| row.get(rc)).map(as_str).unwrap_or_else(|| c.database.clone());
    let role_cols: [&str; 3] = [c.id_col.as_str(), c.label_col.as_str(), c.realm_col.as_deref().unwrap_or("")];
    let props = collect_props(row, &role_cols);
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
    let props = collect_props(row, &role_cols);
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

    /// Per-hop SQL BFS — one `neighbors` query per hop. Retained as the
    /// fallback for graphs whose edge set exceeds [`topo_max_edges`] (where a
    /// whole-topology in-memory build would be too large): the seed-scoped
    /// `WHERE src/dst IN frontier` queries stay bounded regardless of graph size.
    async fn subgraph_per_hop(&self, id: &str, depth: usize) -> anyhow::Result<GraphPayload> {
        let mut frontier = vec![id.to_string()];
        let mut all_node_ids: std::collections::HashSet<String> = frontier.iter().cloned().collect();
        let mut edges: Vec<GraphRelationship> = Vec::new();
        for _ in 0..depth.max(1) {
            if frontier.is_empty() {
                break;
            }
            let exp = self.neighbors(&frontier, Direction::Both, true, 500).await?;
            let mut next = Vec::new();
            for e in exp.edges {
                if !edges.iter().any(|k| k.id == e.id) {
                    edges.push(e);
                }
            }
            for nid in exp.new_node_ids {
                if all_node_ids.insert(nid.clone()) {
                    next.push(nid);
                }
            }
            frontier = next;
        }
        let mut nodes = Vec::new();
        for nid in &all_node_ids {
            if let Some(n) = self.node(nid).await? {
                nodes.push(n);
            }
        }
        let stats = GraphStats {
            total_nodes: nodes.len(),
            total_relationships: edges.len(),
            label_counts: BTreeMap::new(),
            relationship_type_counts: BTreeMap::new(),
        };
        Ok(GraphPayload { stats, nodes, edges })
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
        // Shallow traversals stay on the targeted per-hop path: a depth-1/2
        // explorer click fetches only frontier-adjacent edges (≤2 small
        // queries), which beats scanning the whole edge table. The CSR win is
        // at depth ≥ 3, where the per-hop loop's N+1 and the historical ~2-hop
        // practical ceiling bite.
        if depth <= 2 {
            return self.subgraph_per_hop(id, depth).await;
        }
        // Deep path: if the whole deduped edge set fits the cap, load it in one
        // scan and BFS in RAM — no per-hop N+1, no practical depth limit.
        // `cap + 1` lets us detect "too big" in a single query, in which case we
        // fall back to the bounded per-hop path (large graphs never attempt a
        // full-topology in-memory build).
        let cap = topo_max_edges();
        let edge_rows = self.rows(edge_sample_sql(&self.cfg, cap + 1)).await?;
        if edge_rows.len() > cap {
            return self.subgraph_per_hop(id, depth).await;
        }
        let all_edges: Vec<GraphRelationship> =
            edge_rows.iter().map(|r| row_to_edge(&self.cfg, r)).collect();
        // CSR over the edge endpoints (+ the seed, so an isolated node still
        // yields itself). `CsrGraph::build` dedups ids and drops dangling edges;
        // here every endpoint is a node, so nothing is dropped.
        let csr = CsrGraph::build(
            all_edges
                .iter()
                .flat_map(|e| [e.source_id.as_str(), e.target_id.as_str()])
                .chain(std::iter::once(id)),
            all_edges
                .iter()
                .map(|e| (e.source_id.as_str(), e.target_id.as_str(), e.relationship_type.as_str())),
        );
        // Undirected reachability to `depth`, matching the per-hop path's
        // `Direction::Both`.
        let reached = csr.k_hop(&[id], depth.max(1), TopoDirection::Both, None);
        let reached_ids: std::collections::HashSet<&str> =
            reached.iter().map(|(_, n, _)| n.as_str()).collect();
        // Induced subgraph: every scanned edge whose endpoints are both reached.
        let edges: Vec<GraphRelationship> = all_edges
            .iter()
            .filter(|e| {
                reached_ids.contains(e.source_id.as_str())
                    && reached_ids.contains(e.target_id.as_str())
            })
            .cloned()
            .collect();
        // Materialize node objects (as the per-hop path did).
        let mut nodes = Vec::new();
        for nid in &reached_ids {
            if let Some(n) = self.node(nid).await? {
                nodes.push(n);
            }
        }
        let stats = GraphStats {
            total_nodes: nodes.len(),
            total_relationships: edges.len(),
            label_counts: BTreeMap::new(),
            relationship_type_counts: BTreeMap::new(),
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
        // Append-only data source tables accumulate a fresh row per node on every
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

    #[test]
    fn hex_decode_round_trips() {
        // `{"k":"v"}` as lowercase hex.
        assert_eq!(hex_decode("7b226b223a2276227d").unwrap(), br#"{"k":"v"}"#.to_vec());
        assert!(hex_decode("abc").is_none(), "odd length");
        assert!(hex_decode("zz").is_none(), "non-hex");
    }

    #[test]
    fn decode_props_blob_handles_hex_and_plain_json() {
        // Binary props arrives hex-encoded from arrow-json.
        let hex = serde_json::json!("7b226b223a2276227d");
        let m = decode_props_blob(&hex).expect("hex json");
        assert_eq!(m.get("k").unwrap(), &serde_json::json!("v"));
        // A Utf8 props column arrives as the JSON text directly.
        let plain = serde_json::json!(r#"{"a":1}"#);
        assert_eq!(decode_props_blob(&plain).unwrap().get("a").unwrap(), &serde_json::json!(1));
        // A non-JSON hex string (e.g. a sha) is left undecoded.
        assert!(decode_props_blob(&serde_json::json!("deadbeef")).is_none());
    }

    #[test]
    fn collect_props_merges_blob_and_explicit_wins() {
        // hex of {"language":"python","name":"blob-name"}
        let blob = "7b226c616e6775616765223a22707974686f6e222c226e616d65223a22626c6f622d6e616d65227d";
        let mut row = JsonRow::new();
        row.insert("id".into(), serde_json::json!("file:1"));
        row.insert("labels".into(), serde_json::json!("CodeFile"));
        row.insert("name".into(), serde_json::json!("real-name")); // explicit promoted col
        row.insert("props".into(), serde_json::json!(blob));
        row.insert("__rn".into(), serde_json::json!(1)); // dedup helper — excluded
        let props = collect_props(&row, &["id", "labels"]);
        // decoded blob field surfaces
        assert_eq!(props.get("language").unwrap(), &serde_json::json!("python"));
        // explicit `name` column wins over the blob's `name`
        assert_eq!(props.get("name").unwrap(), &serde_json::json!("real-name"));
        // helper + role cols excluded; raw hex `props` not leaked
        assert!(!props.contains_key("__rn"));
        assert!(!props.contains_key("props"));
        assert!(!props.contains_key("id"));
    }

    #[test]
    fn collect_props_keeps_undecodable_blob_raw() {
        let mut row = JsonRow::new();
        row.insert("props".into(), serde_json::json!("not-json"));
        let props = collect_props(&row, &[]);
        assert_eq!(props.get("props").unwrap(), &serde_json::json!("not-json"));
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

    /// A 5-hop chain a→b→c→d→e→f. Edge queries return the whole chain in one
    /// shot; node-by-id queries echo the requested id (parsed from the literal).
    struct DeepChainExec;
    #[async_trait]
    impl GraphQueryExecutor for DeepChainExec {
        async fn query(&self, _db: &str, sql: String) -> anyhow::Result<Vec<JsonRow>> {
            let s = sql.to_lowercase();
            if s.contains("kg_edges") {
                let chain = [("a", "b"), ("b", "c"), ("c", "d"), ("d", "e"), ("e", "f")];
                return Ok(chain
                    .iter()
                    .map(|(src, dst)| {
                        row(&[
                            ("src", serde_json::json!(src)),
                            ("dst", serde_json::json!(dst)),
                            ("type", serde_json::json!("LINKS")),
                        ])
                    })
                    .collect());
            }
            if s.contains("kg_nodes") {
                // node_by_id_sql: `... "id" = '<x>' limit 1` → echo <x>.
                if let Some(idv) = sql.split('\'').nth(1) {
                    return Ok(vec![row(&[
                        ("id", serde_json::json!(idv)),
                        ("labels", serde_json::json!("N")),
                    ])]);
                }
            }
            Ok(vec![])
        }
    }
    fn deep_provider() -> StoredGraphProvider {
        StoredGraphProvider::new(
            StoredGraphConfig {
                database: "kg".into(), node_table: "kg_nodes".into(), edge_table: "kg_edges".into(),
                id_col: "id".into(), label_col: "labels".into(), src_col: "src".into(),
                dst_col: "dst".into(), type_col: "type".into(), realm_col: None,
            },
            std::sync::Arc::new(DeepChainExec),
        )
    }

    #[tokio::test]
    async fn subgraph_traverses_beyond_two_hops_in_one_scan() {
        let p = deep_provider();
        // depth 5 reaches the whole chain (the old per-hop default capped at 2).
        let g = p.subgraph("a", 5).await.unwrap();
        let ids: std::collections::BTreeSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            ["a", "b", "c", "d", "e", "f"].into_iter().collect(),
            "5-hop reach should include all six nodes"
        );
        assert_eq!(g.edges.len(), 5, "all chain edges are induced at depth 5");
    }

    #[tokio::test]
    async fn subgraph_respects_depth_bound() {
        // depth 3 takes the CSR path (depth > 2) and stops at d on the chain.
        let p = deep_provider();
        let g = p.subgraph("a", 3).await.unwrap();
        let ids: std::collections::BTreeSet<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c", "d"].into_iter().collect(), "depth 3 stops at d");
        assert_eq!(g.edges.len(), 3, "only a→b, b→c, c→d are induced");
    }
}
