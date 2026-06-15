//! Differential test: the `graph-var-length` KQL operator (endpoint-preserving
//! variable-length reachability) executed through DataFusion must agree, set
//! for set, with the petgraph `var_length` oracle — across forward / backward /
//! both directions, edge-type filters, and on cyclic topologies (cycle safety).
//!
//! For each topology we use EVERY node as the seed and compare the engine's
//! `(src, dst, depth)` rows to `oracle::var_length`. This pins the lowering's
//! shortest-depth/node-distinct semantics (MIN over walks up to `max`, then the
//! `[min,max]` window) against the normative reference — no engine, no docker,
//! just DataFusion over an in-memory edge table.

use std::collections::BTreeSet;
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use datafusion::datasource::MemTable;
use datafusion::prelude::SessionContext;
use kyma_graph_testkit::model::TestGraph;
use kyma_graph_testkit::oracle::{self, Direction};
use kyma_graph_testkit::synth;
use kyma_kql::kql_to_sql;

/// Register `g`'s edges as an in-memory `edges(src, dst, type)` table.
fn edges_ctx(g: &TestGraph) -> SessionContext {
    let srcs: Vec<&str> = g.edges.iter().map(|e| e.src.as_str()).collect();
    let dsts: Vec<&str> = g.edges.iter().map(|e| e.dst.as_str()).collect();
    let types: Vec<&str> = g.edges.iter().map(|e| e.etype.as_str()).collect();
    let schema = Arc::new(Schema::new(vec![
        Field::new("src", DataType::Utf8, false),
        Field::new("dst", DataType::Utf8, false),
        Field::new("type", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(srcs)),
            Arc::new(StringArray::from(dsts)),
            Arc::new(StringArray::from(types)),
        ],
    )
    .unwrap();
    let ctx = SessionContext::new();
    ctx.register_table("edges", Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()))
        .unwrap();
    ctx
}

/// Run `graph-var-length` from `seed` and collect `(src, dst, depth)` rows.
async fn engine_pairs(
    g: &TestGraph,
    seed: &str,
    min: usize,
    max: usize,
    dir_kw: &str,
    etype: Option<&str>,
) -> BTreeSet<(String, String, i64)> {
    let ctx = edges_ctx(g);
    let et = etype
        .map(|t| format!(r#" edge-type "{t}""#))
        .unwrap_or_default();
    let kql = format!(
        r#"edges | graph-var-length source "{seed}" from src to dst min-hops {min} max-hops {max} direction {dir_kw}{et}"#
    );
    let sql = kql_to_sql(&kql).expect("lower graph-var-length");
    let batches = ctx.sql(&sql).await.expect("plan").collect().await.expect("exec");
    let mut out = BTreeSet::new();
    for b in &batches {
        let src = b.column_by_name("src").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let dst = b.column_by_name("dst").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let depth = b.column_by_name("depth").unwrap().as_any().downcast_ref::<Int64Array>().unwrap();
        for i in 0..b.num_rows() {
            out.insert((src.value(i).to_string(), dst.value(i).to_string(), depth.value(i)));
        }
    }
    out
}

/// Compare engine vs oracle using every node as the seed.
async fn assert_matches_all_seeds(
    g: &TestGraph,
    min: usize,
    max: usize,
    dir: Direction,
    dir_kw: &str,
    etype: Option<&str>,
) {
    for n in &g.nodes {
        let seed = n.id.as_str();
        let want: BTreeSet<(String, String, i64)> = oracle::var_length(g, &[seed], min, max, dir, etype)
            .into_iter()
            .map(|(s, d, dep)| (s, d, dep as i64))
            .collect();
        let got = engine_pairs(g, seed, min, max, dir_kw, etype).await;
        assert_eq!(
            got, want,
            "graph-var-length != oracle: seed={seed} min={min} max={max} dir={dir_kw} etype={etype:?}"
        );
    }
}

#[tokio::test]
async fn grid_forward_windows_match_oracle() {
    let g = synth::grid(4, 4);
    assert_matches_all_seeds(&g, 1, 2, Direction::Fwd, "forward", None).await;
    assert_matches_all_seeds(&g, 2, 3, Direction::Fwd, "forward", None).await;
    // min=0 keeps the depth-0 self row.
    assert_matches_all_seeds(&g, 0, 2, Direction::Fwd, "forward", None).await;
}

#[tokio::test]
async fn grid_backward_and_both_match_oracle() {
    let g = synth::grid(4, 4);
    assert_matches_all_seeds(&g, 1, 3, Direction::Back, "backward", None).await;
    assert_matches_all_seeds(&g, 1, 2, Direction::Both, "both", None).await;
}

#[tokio::test]
async fn grid_edge_type_filter_matches_oracle() {
    let g = synth::grid(4, 4);
    // Only the horizontal LINKS axis — exercises the per-hop type filter.
    assert_matches_all_seeds(&g, 1, 3, Direction::Fwd, "forward", Some("LINKS")).await;
}

#[tokio::test]
async fn cyclic_is_cycle_safe_and_dedups_to_shortest() {
    // A directed 5-ring: even with max far past the ring size, each node is
    // reported once at its shortest depth (the recursion terminates on cycles).
    let g = synth::cyclic(&[5], 0, 1);
    assert_matches_all_seeds(&g, 1, 10, Direction::Fwd, "forward", None).await;
    assert_matches_all_seeds(&g, 2, 4, Direction::Both, "both", None).await;
}

#[tokio::test]
async fn hub_and_spoke_matches_oracle() {
    let g = synth::hub_and_spoke(2, 4);
    assert_matches_all_seeds(&g, 1, 2, Direction::Fwd, "forward", None).await;
    assert_matches_all_seeds(&g, 1, 2, Direction::Both, "both", None).await;
}

// =====================================================================
// graph-var-match: open (all-sources) endpoint-pair binding with node joins.
// This is what Cypher MATCH (a)-[*M..N]->(b) lowers onto, so the differential
// is against oracle::var_length over EVERY node as a source.
// =====================================================================

/// Register both `edges(src,dst,type)` and `nodes(id,name)` for `g`.
fn nodes_and_edges_ctx(g: &TestGraph) -> SessionContext {
    let ctx = edges_ctx(g);
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let names: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(ids)),
            Arc::new(StringArray::from(names)),
        ],
    )
    .unwrap();
    ctx.register_table("nodes", Arc::new(MemTable::try_new(schema, vec![vec![batch]]).unwrap()))
        .unwrap();
    ctx
}

/// Run graph-var-match (all sources) and collect bound `(a_id, b_id)` pairs.
async fn varmatch_pairs(
    g: &TestGraph,
    min: usize,
    max: usize,
    dir_kw: &str,
    etype: Option<&str>,
) -> BTreeSet<(String, String)> {
    let ctx = nodes_and_edges_ctx(g);
    let rel = etype.map(|t| format!("[r:{t}]")).unwrap_or_else(|| "[r]".to_string());
    let kql = format!(
        "edges | make-graph src --> dst with nodes on id \
         | graph-var-match (a)-{rel}->(b) min-hops {min} max-hops {max} direction {dir_kw} \
         project a.id as a_id, b.id as b_id"
    );
    let sql = kql_to_sql(&kql).expect("lower graph-var-match");
    let batches = ctx.sql(&sql).await.expect("plan").collect().await.expect("exec");
    let mut out = BTreeSet::new();
    for b in &batches {
        let a = b.column_by_name("a_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let bb = b.column_by_name("b_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        for i in 0..b.num_rows() {
            out.insert((a.value(i).to_string(), bb.value(i).to_string()));
        }
    }
    out
}

/// Oracle endpoint pairs over ALL sources (depth projected away).
fn oracle_all_pairs(
    g: &TestGraph,
    min: usize,
    max: usize,
    dir: Direction,
    etype: Option<&str>,
) -> BTreeSet<(String, String)> {
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    oracle::var_length(g, &ids, min, max, dir, etype)
        .into_iter()
        .map(|(s, d, _)| (s, d))
        .collect()
}

async fn assert_varmatch(
    g: &TestGraph,
    min: usize,
    max: usize,
    dir: Direction,
    dir_kw: &str,
    etype: Option<&str>,
) {
    let got = varmatch_pairs(g, min, max, dir_kw, etype).await;
    let want = oracle_all_pairs(g, min, max, dir, etype);
    assert_eq!(
        got, want,
        "graph-var-match != oracle (all sources): min={min} max={max} dir={dir_kw} etype={etype:?}"
    );
}

#[tokio::test]
async fn var_match_open_endpoint_pairs_match_oracle() {
    let g = synth::grid(4, 4);
    assert_varmatch(&g, 1, 2, Direction::Fwd, "forward", None).await;
    assert_varmatch(&g, 2, 3, Direction::Fwd, "forward", None).await;
    assert_varmatch(&g, 1, 3, Direction::Back, "backward", None).await;
    assert_varmatch(&g, 1, 2, Direction::Both, "both", None).await;
    assert_varmatch(&g, 1, 3, Direction::Fwd, "forward", Some("LINKS")).await;
}

#[tokio::test]
async fn var_match_on_cycles_matches_oracle() {
    let g = synth::cyclic(&[5, 4], 2, 7);
    assert_varmatch(&g, 1, 8, Direction::Fwd, "forward", None).await;
    assert_varmatch(&g, 2, 4, Direction::Both, "both", None).await;
}

// =====================================================================
// graph-shortest-path: the operator Cypher `shortestPath(...)` lowers onto.
// Differential vs oracle::shortest_path_len over node pairs.
// =====================================================================

use kyma_graph_testkit::oracle::shortest_path_len;

/// Shortest-path length `src`→`tgt` via the operator (None when no path within
/// `max` hops).
async fn engine_sp(
    g: &TestGraph,
    src: &str,
    tgt: &str,
    max: usize,
    dir_kw: &str,
) -> Option<i64> {
    let ctx = edges_ctx(g);
    let kql = format!(
        "edges | graph-shortest-path source \"{src}\" target \"{tgt}\" \
         from src to dst max-hops {max} direction {dir_kw} | project depth"
    );
    let sql = kql_to_sql(&kql).expect("lower graph-shortest-path");
    let batches = ctx.sql(&sql).await.expect("plan").collect().await.expect("exec");
    for b in &batches {
        if b.num_rows() == 0 {
            continue;
        }
        let d = b.column_by_name("depth").unwrap().as_any().downcast_ref::<Int64Array>().unwrap();
        return if d.is_null(0) { None } else { Some(d.value(0)) };
    }
    None
}

/// With `max` large enough that every reachable pair is within it, the engine
/// distance equals the oracle for all node pairs (and None ⇔ unreachable).
async fn assert_sp_all_pairs(g: &TestGraph, max: usize, dir: Direction, dir_kw: &str) {
    for s in &g.nodes {
        for t in &g.nodes {
            let got = engine_sp(g, &s.id, &t.id, max, dir_kw).await.map(|d| d as usize);
            let want = shortest_path_len(g, &s.id, &t.id, dir, None);
            assert_eq!(
                got, want,
                "shortest-path {}→{} dir={dir_kw}: engine {got:?} vs oracle {want:?}",
                s.id, t.id
            );
        }
    }
}

#[tokio::test]
async fn shortest_path_matches_oracle_on_grid() {
    let g = synth::grid(3, 3);
    // max=8 exceeds the grid diameter, so reachable ⇔ within max.
    assert_sp_all_pairs(&g, 8, Direction::Fwd, "forward").await;
    assert_sp_all_pairs(&g, 8, Direction::Both, "both").await;
}

#[tokio::test]
async fn shortest_path_matches_oracle_on_cycle() {
    let g = synth::cyclic(&[5], 0, 1);
    assert_sp_all_pairs(&g, 12, Direction::Fwd, "forward").await;
    assert_sp_all_pairs(&g, 12, Direction::Both, "both").await;
}

// =====================================================================
// Multi-segment graph-match: non-linear (star) pattern join.
// =====================================================================

#[tokio::test]
async fn multi_segment_star_graph_match_executes_correctly() {
    // Star from `a`: (a)-[r]->(b), (a)-[s]->(c). With a's out-neighbours
    // {b, c, d}, the result is their cartesian product on (b, c).
    let mut g = TestGraph::new();
    for id in ["a", "b", "c", "d"] {
        g.add_node(id, "N");
    }
    g.add_edge("a", "b", "R");
    g.add_edge("a", "c", "R");
    g.add_edge("a", "d", "R");

    let ctx = nodes_and_edges_ctx(&g);
    let kql = "edges | make-graph src --> dst with nodes on id \
               | graph-match (a)-[r]->(b), (a)-[s]->(c) \
               project a.id as a_id, b.id as b_id, c.id as c_id";
    let sql = kql_to_sql(kql).expect("lower multi-segment graph-match");
    let batches = ctx.sql(&sql).await.expect("plan").collect().await.expect("exec");
    let mut got: BTreeSet<(String, String, String)> = BTreeSet::new();
    for bt in &batches {
        let a = bt.column_by_name("a_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let b = bt.column_by_name("b_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let c = bt.column_by_name("c_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        for i in 0..bt.num_rows() {
            got.insert((a.value(i).into(), b.value(i).into(), c.value(i).into()));
        }
    }
    let mut want: BTreeSet<(String, String, String)> = BTreeSet::new();
    for x in ["b", "c", "d"] {
        for y in ["b", "c", "d"] {
            want.insert(("a".into(), x.into(), y.into()));
        }
    }
    assert_eq!(got, want, "star-pattern join = cartesian product of a's neighbours");
}

/// Run `(a)-[r]->(b), OPTIONAL (b)-[s]->(c)` and collect `(a, b, Option<c>)`.
async fn optional_rows(g: &TestGraph) -> Vec<(String, String, Option<String>)> {
    let ctx = nodes_and_edges_ctx(g);
    let kql = "edges | make-graph src --> dst with nodes on id \
               | graph-match (a)-[r]->(b), OPTIONAL (b)-[s]->(c) \
               project a.id as a_id, b.id as b_id, c.id as c_id";
    let sql = kql_to_sql(kql).expect("lower OPTIONAL graph-match");
    let batches = ctx.sql(&sql).await.expect("plan").collect().await.expect("exec");
    let mut rows = Vec::new();
    for bt in &batches {
        let a = bt.column_by_name("a_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let b = bt.column_by_name("b_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        let c = bt.column_by_name("c_id").unwrap().as_any().downcast_ref::<StringArray>().unwrap();
        for i in 0..bt.num_rows() {
            rows.push((
                a.value(i).to_string(),
                b.value(i).to_string(),
                if c.is_null(i) { None } else { Some(c.value(i).to_string()) },
            ));
        }
    }
    rows.sort();
    rows
}

#[tokio::test]
async fn optional_match_left_joins_with_nulls() {
    // b has no outgoing edge → the OPTIONAL leg is unmatched → c is NULL, but
    // the (a, b) row survives (LEFT JOIN, not inner).
    let mut g = TestGraph::new();
    for id in ["a", "b"] {
        g.add_node(id, "N");
    }
    g.add_edge("a", "b", "R");
    assert_eq!(
        optional_rows(&g).await,
        vec![("a".into(), "b".into(), None)],
        "unmatched OPTIONAL ⇒ NULL c, row preserved"
    );

    // Add b→c. The mandatory (a)-[r]->(b) now matches BOTH edges (a,b are
    // unbound): a→b then b→c matches the OPTIONAL leg (c present); b→c then c
    // has no outgoing edge (c NULL). Both rows survive — exactly OPTIONAL's
    // matched-and-unmatched-in-one-query semantics.
    let mut g2 = TestGraph::new();
    for id in ["a", "b", "c"] {
        g2.add_node(id, "N");
    }
    g2.add_edge("a", "b", "R");
    g2.add_edge("b", "c", "R");
    assert_eq!(
        optional_rows(&g2).await,
        vec![
            ("a".into(), "b".into(), Some("c".into())),
            ("b".into(), "c".into(), None),
        ],
        "matched leg ⇒ c present; unmatched leg ⇒ NULL c"
    );
}
