# Graph Layer G1b.2 — StoredGraphProvider + Server Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Serve **registered** property-graphs (G1b.1) through the same `/v1/graph/*` endpoints and `/graph` UI as the synthetic schema-graph — by querying their node/edge tables and shaping rows into the wire types.

**Architecture:** Keep `kyma-graph` decoupled from the engine. Add a narrow `GraphQueryExecutor` trait (run SQL against a database → JSON rows) — mirroring how `SchemaSource` decoupled G1a. `StoredGraphProvider` holds a plain `StoredGraphConfig` (column roles + table names, mapped from the catalog's `GraphRegistration`) + an executor, builds SQL, and shapes rows. `kyma-server` implements the executor over the existing `SessionContext`/`KymaTable` query path (the `execute_sql` pattern in `agent/tools.rs:341`) and routes `/v1/graph/{name}` to schema-vs-stored by catalog lookup keyed on the `X-Database` header.

**Tech Stack:** Rust, `async-trait`, `serde_json`, DataFusion (server-side only).

**Reference:** `crates/kyma-graph/src/{provider,schema_graph,types}.rs` (existing patterns), `crates/kyma-server/src/graph_handler.rs` (handlers/router), `crates/kyma-server/src/agent/tools.rs:341` `execute_sql` (SessionContext build + Arrow→JSON), `kyma_core::catalog::GraphRegistration` (G1b.1).

**Working dir:** worktree `…/.claude/worktrees/feature+graph-layer`. Docker for tests.

---

## Task 1: `GraphQueryExecutor` trait + `StoredGraphConfig` (kyma-graph)

**Files:** create `crates/kyma-graph/src/executor.rs`; modify `crates/kyma-graph/src/lib.rs`.

- [ ] **Step 1: `crates/kyma-graph/src/executor.rs`:**
```rust
//! The seam that lets the stored-graph provider run SQL over node/edge tables
//! without `kyma-graph` depending on the engine. `kyma-server` implements this
//! over its DataFusion query path.

use async_trait::async_trait;

/// One result row: column name -> JSON value.
pub type JsonRow = serde_json::Map<String, serde_json::Value>;

#[async_trait]
pub trait GraphQueryExecutor: Send + Sync {
    /// Run `sql` against `database`'s tables, returning rows as JSON objects.
    async fn query(&self, database: &str, sql: String) -> anyhow::Result<Vec<JsonRow>>;
}

/// Column roles + table names for a registered graph (decoupled mirror of the
/// catalog's `GraphRegistration`). The server maps registration → this.
#[derive(Debug, Clone)]
pub struct StoredGraphConfig {
    pub database: String,
    pub node_table: String,
    pub edge_table: String,
    pub id_col: String,
    pub label_col: String,
    pub src_col: String,
    pub dst_col: String,
    pub type_col: String,
    pub realm_col: Option<String>,
}
```

- [ ] **Step 2:** in `lib.rs` add `pub mod executor;` and `pub use executor::{GraphQueryExecutor, JsonRow, StoredGraphConfig};`. Also add `pub use stored_graph::StoredGraphProvider;` (the module created in Task 2 — add it now; the build will be green after Task 2). To keep this task building on its own, create a stub `crates/kyma-graph/src/stored_graph.rs` with just `//! Stored-graph provider (Task 2).` and DON'T add the `pub use stored_graph::StoredGraphProvider;` line yet (add it in Task 2). Add `pub mod stored_graph;`.

- [ ] **Step 3:** `cargo build -p kyma-graph` clean. Commit:
```bash
git add crates/kyma-graph/src/executor.rs crates/kyma-graph/src/lib.rs crates/kyma-graph/src/stored_graph.rs
git commit -m "feat(graph): GraphQueryExecutor seam + StoredGraphConfig"
```

---

## Task 2: SQL builders + row shaping (pure, unit-tested)

**Files:** `crates/kyma-graph/src/stored_graph.rs`.

These are pure functions so they're testable without an executor. **Identifier quoting:** wrap table/column names in double quotes. **Literal escaping:** single-quote values via `'` → `''` (helper `lit`). Node ids etc. come from trusted catalog data but escape anyway.

- [ ] **Step 1: failing tests** — create the test module first:
```rust
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
```

- [ ] **Step 2:** run `cargo test -p kyma-graph stored_graph::sql_tests` → FAIL.

- [ ] **Step 3: implement** (prepend above the test module). Replace the stub file's contents:
```rust
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

pub(crate) fn node_sample_sql(c: &StoredGraphConfig, limit: usize) -> String {
    format!("select * from {} limit {}", ident(&c.node_table), limit)
}
pub(crate) fn edge_sample_sql(c: &StoredGraphConfig, limit: usize) -> String {
    format!("select * from {} limit {}", ident(&c.edge_table), limit)
}
pub(crate) fn node_by_id_sql(c: &StoredGraphConfig, id: &str) -> String {
    format!("select * from {} where {} = {} limit 1", ident(&c.node_table), ident(&c.id_col), lit(id))
}
pub(crate) fn neighbors_sql(c: &StoredGraphConfig, ids: &[String], dir: Direction, limit: usize) -> String {
    let list = in_list(ids);
    let pred = match dir {
        Direction::Forward => format!("{} in ({list})", ident(&c.src_col)),
        Direction::Backward => format!("{} in ({list})", ident(&c.dst_col)),
        Direction::Both => format!("{} in ({list}) or {} in ({list})", ident(&c.src_col), ident(&c.dst_col)),
    };
    format!("select * from {} where {pred} limit {limit}", ident(&c.edge_table))
}
pub(crate) fn search_sql(c: &StoredGraphConfig, text: &str, limit: usize, offset: usize) -> String {
    let needle = lit(&format!("%{}%", text.to_lowercase()));
    format!(
        "select * from {t} where lower(cast({id} as varchar)) like {n} or lower(cast({lbl} as varchar)) like {n} limit {limit} offset {offset}",
        t = ident(&c.node_table), id = ident(&c.id_col), lbl = ident(&c.label_col), n = needle,
    )
}
pub(crate) fn count_sql(table: &str) -> String {
    format!("select count(*) as n from {}", ident(table))
}
pub(crate) fn group_count_sql(table: &str, col: &str) -> String {
    format!("select cast({c} as varchar) as k, count(*) as n from {t} group by {c}", c = ident(col), t = ident(table))
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
        if role_cols.contains(&k.as_str()) { continue; }
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
        if role_cols.contains(&k.as_str()) { continue; }
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
    let total_nodes = StoredGraphProvider::count_of(&p.rows(count_sql(&p.cfg.node_table)).await?);
    let total_relationships = StoredGraphProvider::count_of(&p.rows(count_sql(&p.cfg.edge_table)).await?);
    let mut label_counts = BTreeMap::new();
    for r in p.rows(group_count_sql(&p.cfg.node_table, &p.cfg.label_col)).await? {
        let k = r.get("k").map(as_str).unwrap_or_default();
        let n = r.get("n").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        label_counts.insert(k, n);
    }
    let mut relationship_type_counts = BTreeMap::new();
    for r in p.rows(group_count_sql(&p.cfg.edge_table, &p.cfg.type_col)).await? {
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
```

- [ ] **Step 4: provider tests with a fake executor** — append a second test module:
```rust
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
```

- [ ] **Step 5:** in `lib.rs` add the re-export `pub use stored_graph::StoredGraphProvider;`. Run `cargo test -p kyma-graph` → all pass; `cargo build -p kyma-graph` + clippy clean.

- [ ] **Step 6: Commit:**
```bash
git add crates/kyma-graph/src/stored_graph.rs crates/kyma-graph/src/lib.rs
git commit -m "feat(graph): StoredGraphProvider (SQL builders + row shaping) over the executor"
```

---

## Task 3: `GraphQueryExecutor` impl + routing in kyma-server

**Files:** `crates/kyma-server/src/graph_handler.rs`.

- [ ] **Step 1: the executor** — add a `QueryEngineExecutor` that mirrors `agent/tools.rs::execute_sql` (READ that fn): build a `SessionContext`, register every `KymaTable` in `database`, run `ctx.sql(sql)`, collect batches, convert each row to a `serde_json::Map` (use the same `arrow::json` writer pattern `execute_sql` uses, then split the array into rows). Hold `catalog: Arc<dyn Catalog>` + `format: Arc<dyn SegmentFormat>` (both on `QueryState`).
```rust
use kyma_graph::{GraphQueryExecutor, JsonRow};

struct QueryEngineExecutor {
    catalog: std::sync::Arc<dyn kyma_core::catalog::Catalog>,
    format: std::sync::Arc<dyn kyma_core::segment_format::SegmentFormat>,
}

#[async_trait]
impl GraphQueryExecutor for QueryEngineExecutor {
    async fn query(&self, database: &str, sql: String) -> anyhow::Result<Vec<JsonRow>> {
        // mirror execute_sql: SessionContext + register KymaTable per table, run sql,
        // serialize batches to JSON via arrow::json::ArrayWriter, return rows as objects.
        // (Implement per the execute_sql pattern; return anyhow::Error on failure.)
        todo!("implement per agent/tools.rs execute_sql; see plan")
    }
}
```
Implement the body for real (the `todo!` is a placeholder marker — replace it). Confirm the exact `SegmentFormat` path/type from `QueryState` (`state.format`).

- [ ] **Step 2: resolve provider by name + database** — replace the synchronous `provider(state)` + `ensure_schema` with an async resolver. Add `HeaderMap` to each handler to read `x-database` (fall back to a `?database=` query param or the empty string). Resolver:
```rust
enum ResolvedProvider { Schema(SchemaGraphProvider), Stored(StoredGraphProvider) }

async fn resolve(state: &QueryState, graph: &str, database: &str) -> Result<ResolvedProvider, Response> {
    if graph == SCHEMA_GRAPH {
        return Ok(ResolvedProvider::Schema(SchemaGraphProvider::new(std::sync::Arc::new(CatalogSchemaSource::new(state.catalog.clone())))));
    }
    match state.catalog.get_graph(database, graph).await {
        Ok(Some(reg)) => {
            let cfg = kyma_graph::StoredGraphConfig {
                database: reg.database, node_table: reg.node_table, edge_table: reg.edge_table,
                id_col: reg.id_col, label_col: reg.label_col, src_col: reg.src_col, dst_col: reg.dst_col,
                type_col: reg.type_col, realm_col: reg.realm_col,
            };
            let exec = std::sync::Arc::new(QueryEngineExecutor { catalog: state.catalog.clone(), format: state.format.clone() });
            Ok(ResolvedProvider::Stored(StoredGraphProvider::new(cfg, exec)))
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, Json(serde_json::json!({"error":{"code":"not_found","message":"unknown graph"}}))).into_response()),
        Err(e) => Err(err500(anyhow::anyhow!(e.to_string()))),
    }
}
```
Add a small helper to call a `GraphProvider` method on either variant (match + dispatch), or `impl GraphProvider for ResolvedProvider` delegating each method. The latter is cleaner — add it.

- [ ] **Step 3: thread `x-database` into handlers.** Each handler gains `headers: axum::http::HeaderMap`; extract `let db = headers.get("x-database").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();` and pass to `resolve`. Update every handler (overview/stats/schema/node/subgraph/search/neighbors) to `match resolve(&state, &graph, &db).await { Ok(p) => /* call p.method */, Err(r) => return r }`.

- [ ] **Step 4: `GET /v1/graph` lists schema + registered** for the request's database:
```rust
async fn list_graphs(State(state): State<QueryState>, headers: axum::http::HeaderMap) -> Response {
    let db = headers.get("x-database").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let mut refs = vec![GraphRef { name: "schema".into(), kind: "schema".into(), description: "Catalog schema as a property-graph.".into() }];
    if !db.is_empty() {
        if let Ok(regs) = state.catalog.list_graphs(&db).await {
            for r in regs {
                refs.push(GraphRef { name: r.name, kind: "stored".into(), description: format!("nodes={}, edges={}", r.node_table, r.edge_table) });
            }
        }
    }
    (StatusCode::OK, Json(refs)).into_response()
}
```

- [ ] **Step 5: integration test** — register a graph over real node/edge tables, ingest a couple rows via the catalog/ingest path, hit the endpoints. Simplest: in the test, create two tables `kg_nodes(id,labels)` / `kg_edges(src,dst,type)` through the catalog + ingest a row each (reuse how other tests ingest, or insert via the staging/ingest API on `QueryState`), `catalog.create_graph("obs","kg", spec)`, then call `resolve` + `overview`. If ingesting rows in-test is heavy, assert the simpler path: registered graph resolves and `stats()`/`overview()` return without error against empty tables (total_nodes==0), and `GET /v1/graph` (with `x-database: obs`) lists the `kg` stored graph. Write whichever is reliable; the must-have assertion is **`GET /v1/graph` lists a registered graph and `/v1/graph/kg/stats` returns 200** (proves routing + executor wiring).

- [ ] **Step 6:** `cargo build -p kyma-server`; `cargo test -p kyma-server --features test-support graph_handler::` all pass; clippy clean. Commit:
```bash
git add crates/kyma-server/src/graph_handler.rs
git commit -m "feat(graph): route /v1/graph/{name} to stored vs schema provider + list registered"
```

---

## Task 4: verify
- [ ] `cargo build -p kyma-graph -p kyma-server` clean.
- [ ] `cargo test -p kyma-graph` (SQL + provider tests) + `cargo test -p kyma-server --features test-support graph_handler::` all pass.
- [ ] `cargo clippy -p kyma-graph -p kyma-server 2>&1 | tail -20` — no new warnings from changed files.

---

## Self-review notes
- **Decoupling preserved:** `kyma-graph` gains no engine dependency; `StoredGraphConfig` mirrors the catalog registration, mapped server-side. `GraphQueryExecutor` is the only new seam.
- **SQL safety:** identifiers double-quoted, literals single-quote-escaped (`lit`). Inputs are catalog/UI-supplied; escaping defends against quote injection regardless.
- **`X-Database` scoping:** stored-graph resolution + listing are scoped to the request's `x-database` header (the UI already sends it). `schema` ignores it (spans the catalog) — unchanged.
- **`ResolvedProvider` impls `GraphProvider`** so handlers call one method regardless of provider kind.
- **Out of scope:** UI changes (the graph selector already calls `GET /v1/graph` and will show registered graphs once they're listed — no web change needed for them to appear; selecting one fetches `/v1/graph/{name}/overview` which now works). Value-based schema-graph edges, G2 KQL operators, G3 perf — later phases.
- **Subgraph** uses iterative `neighbors` (depth-bounded) — fine for v1; G3 optimizes traversal.
