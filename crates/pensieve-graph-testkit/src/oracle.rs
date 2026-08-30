//! petgraph-backed reference engine.
//!
//! **These doc comments are the spec.** The S3 vectorized path operator
//! (k-hop / var-length / shortest-path / backward expansion) and the existing
//! Cypher `MATCH`-chain subset are differentially validated against exactly
//! the semantics written here. When in doubt, the oracle is right and the
//! engine is wrong.
//!
//! Shared conventions:
//! - Graphs are **directed multigraphs**: parallel edges (same `src`/`dst`,
//!   any types) are all kept and traversable.
//! - An optional `etype` filter restricts traversal to edges whose `etype`
//!   compares **equal** to the filter; `None` traverses every edge.
//! - Edges whose `src` or `dst` id has no row in the node set are **dropped
//!   before any traversal** — the engine inner-joins the node table on both
//!   endpoints, so a dangling edge can never produce a result row.
//! - Node ids are opaque strings; results are sets of id tuples, never
//!   row-ordered (the engine is free to return rows in any order and with
//!   duplicates; comparison is set-based).

use std::collections::{BTreeSet, HashMap, VecDeque};

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction as PgDir;

use crate::model::TestGraph;

/// Traversal direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Follow edges `src → dst`.
    Fwd,
    /// Follow edges `dst → src` (reverse traversal).
    Back,
    /// Treat every edge as bidirectional (shortest hop count in the
    /// underlying undirected multigraph, after the `etype` filter).
    Both,
}

/// Prebuilt petgraph view of a [`TestGraph`] — build once, query many times.
///
/// Node weights are indices into `TestGraph::nodes`; edge weights are indices
/// into `TestGraph::edges` (so the `etype` filter reads the original edge).
pub struct Oracle<'g> {
    src: &'g TestGraph,
    graph: DiGraph<usize, usize>,
    /// node id → petgraph index.
    idx: HashMap<&'g str, NodeIndex>,
}

impl<'g> Oracle<'g> {
    /// Build the petgraph mirror. Edges with endpoints missing from the node
    /// set are dropped here (see module docs).
    pub fn new(g: &'g TestGraph) -> Self {
        let mut graph = DiGraph::with_capacity(g.nodes.len(), g.edges.len());
        let mut idx: HashMap<&str, NodeIndex> = HashMap::with_capacity(g.nodes.len());
        for (i, n) in g.nodes.iter().enumerate() {
            let ni = graph.add_node(i);
            idx.insert(n.id.as_str(), ni);
        }
        for (i, e) in g.edges.iter().enumerate() {
            if let (Some(&s), Some(&d)) = (idx.get(e.src.as_str()), idx.get(e.dst.as_str())) {
                graph.add_edge(s, d, i);
            }
        }
        Self { src: g, graph, idx }
    }

    fn node_id(&self, ni: NodeIndex) -> &'g str {
        self.src.nodes[self.graph[ni]].id.as_str()
    }

    /// Neighbors of `ni` reachable over one edge in `dir`, honoring `etype`.
    fn step(&self, ni: NodeIndex, dir: Direction, etype: Option<&str>) -> Vec<NodeIndex> {
        use petgraph::visit::EdgeRef as _;
        let collect = |pg_dir: PgDir, v: &mut Vec<NodeIndex>| {
            for er in self.graph.edges_directed(ni, pg_dir) {
                let edge = &self.src.edges[*er.weight()];
                if etype.is_none_or(|t| edge.etype == t) {
                    v.push(match pg_dir {
                        PgDir::Outgoing => er.target(),
                        PgDir::Incoming => er.source(),
                    });
                }
            }
        };
        let mut out = Vec::new();
        match dir {
            Direction::Fwd => collect(PgDir::Outgoing, &mut out),
            Direction::Back => collect(PgDir::Incoming, &mut out),
            Direction::Both => {
                collect(PgDir::Outgoing, &mut out);
                collect(PgDir::Incoming, &mut out);
            }
        }
        out
    }

    /// k-hop expansion — see [`k_hop`] for the normative semantics.
    pub fn k_hop(
        &self,
        sources: &[&str],
        k: usize,
        dir: Direction,
        etype: Option<&str>,
    ) -> BTreeSet<(String, String, usize)> {
        let mut out = BTreeSet::new();
        for &s in sources {
            let Some(&start) = self.idx.get(s) else {
                continue; // unknown source id → contributes nothing
            };
            // Node-distinct BFS per source: each node is reported once, at its
            // shortest hop count from this source. Cycle-safe by construction.
            let mut dist: HashMap<NodeIndex, usize> = HashMap::new();
            dist.insert(start, 0);
            out.insert((s.to_string(), s.to_string(), 0));
            let mut q = VecDeque::from([start]);
            while let Some(cur) = q.pop_front() {
                let d = dist[&cur];
                if d == k {
                    continue;
                }
                for nb in self.step(cur, dir, etype) {
                    if !dist.contains_key(&nb) {
                        dist.insert(nb, d + 1);
                        out.insert((s.to_string(), self.node_id(nb).to_string(), d + 1));
                        q.push_back(nb);
                    }
                }
            }
        }
        out
    }

    /// Var-length expansion — see [`var_length`] for the normative semantics.
    pub fn var_length(
        &self,
        sources: &[&str],
        min: usize,
        max: usize,
        dir: Direction,
        etype: Option<&str>,
    ) -> BTreeSet<(String, String, usize)> {
        self.k_hop(sources, max, dir, etype)
            .into_iter()
            .filter(|(_, _, d)| *d >= min)
            .collect()
    }

    /// Unweighted shortest path — see [`shortest_path_len`] for semantics.
    pub fn shortest_path_len(
        &self,
        src: &str,
        dst: &str,
        dir: Direction,
        etype: Option<&str>,
    ) -> Option<usize> {
        let (&start, &goal) = (self.idx.get(src)?, self.idx.get(dst)?);
        if start == goal {
            return Some(0);
        }
        let mut dist: HashMap<NodeIndex, usize> = HashMap::new();
        dist.insert(start, 0);
        let mut q = VecDeque::from([start]);
        while let Some(cur) = q.pop_front() {
            let d = dist[&cur];
            for nb in self.step(cur, dir, etype) {
                if nb == goal {
                    return Some(d + 1);
                }
                if !dist.contains_key(&nb) {
                    dist.insert(nb, d + 1);
                    q.push_back(nb);
                }
            }
        }
        None
    }

    /// Forward MATCH-chain evaluation — see [`match_chain`] for semantics.
    pub fn match_chain(&self, pattern: &ChainPattern) -> BTreeSet<Vec<String>> {
        assert_eq!(
            pattern.nodes.len(),
            pattern.edges.len() + 1,
            "ChainPattern: need exactly one more node spec than edge specs"
        );
        let mut out = BTreeSet::new();
        let label_ok = |ni: NodeIndex, spec: &NodeSpec| -> bool {
            match &spec.label {
                None => true,
                Some(l) => self.src.nodes[self.graph[ni]].primary_label() == Some(l.as_str()),
            }
        };
        // DFS over pattern positions. Out-degree-bounded topologies keep this
        // tractable; the engine does the same joins in SQL.
        let mut stack: Vec<NodeIndex> = Vec::with_capacity(pattern.nodes.len());
        for start in self.graph.node_indices() {
            if !label_ok(start, &pattern.nodes[0]) {
                continue;
            }
            stack.push(start);
            self.extend_chain(pattern, &mut stack, &mut out, label_ok);
            stack.pop();
        }
        out
    }

    fn extend_chain(
        &self,
        pattern: &ChainPattern,
        stack: &mut Vec<NodeIndex>,
        out: &mut BTreeSet<Vec<String>>,
        label_ok: impl Fn(NodeIndex, &NodeSpec) -> bool + Copy,
    ) {
        let pos = stack.len() - 1; // index of the last bound node
        if pos == pattern.edges.len() {
            out.insert(stack.iter().map(|&ni| self.node_id(ni).to_string()).collect());
            return;
        }
        let cur = *stack.last().expect("stack non-empty");
        let etype = pattern.edges[pos].etype.as_deref();
        for nb in self.step(cur, Direction::Fwd, etype) {
            if label_ok(nb, &pattern.nodes[pos + 1]) {
                stack.push(nb);
                self.extend_chain(pattern, stack, out, label_ok);
                stack.pop();
            }
        }
    }
}

// =====================================================================
// Chain pattern (the differential currency for the existing Cypher subset)
// =====================================================================

/// Node position in a [`ChainPattern`]: optional `:Label` filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeSpec {
    /// `Some("Hub")` ⇒ the bound node's **primary label** (the string stored
    /// in the engine's `labels` column) must equal `"Hub"` exactly.
    pub label: Option<String>,
}

/// Edge position in a [`ChainPattern`]: optional `[:TYPE]` filter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EdgeSpec {
    /// `Some("LINKS")` ⇒ the bound edge's `etype` must equal `"LINKS"`.
    pub etype: Option<String>,
}

/// A forward MATCH chain `(n0[:L0])-[r0[:T0]]->(n1[:L1])-…->(nK)`.
///
/// Invariant: `nodes.len() == edges.len() + 1`, `edges.len() ≥ 1`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainPattern {
    pub nodes: Vec<NodeSpec>,
    pub edges: Vec<EdgeSpec>,
}

impl ChainPattern {
    /// Start a builder with the first node spec.
    pub fn start(label: Option<&str>) -> Self {
        Self {
            nodes: vec![NodeSpec {
                label: label.map(str::to_string),
            }],
            edges: Vec::new(),
        }
    }

    /// Append one hop: an edge spec followed by the next node spec.
    #[must_use]
    pub fn hop(mut self, etype: Option<&str>, label: Option<&str>) -> Self {
        self.edges.push(EdgeSpec {
            etype: etype.map(str::to_string),
        });
        self.nodes.push(NodeSpec {
            label: label.map(str::to_string),
        });
        self
    }

    /// Render as a Cypher query the engine's `application/x-cypher` frontend
    /// accepts. Variables are `n0..nK` / `r0..r{K-1}`; the projection is the
    /// node id tuple (`RETURN n0.id, n1.id, …`), which the engine returns as
    /// NDJSON columns `n0_id, n1_id, …`.
    pub fn to_cypher(&self) -> String {
        let mut q = String::from("MATCH ");
        for (i, node) in self.nodes.iter().enumerate() {
            if i > 0 {
                let e = &self.edges[i - 1];
                match &e.etype {
                    Some(t) => q.push_str(&format!("-[r{}:{t}]->", i - 1)),
                    None => q.push_str(&format!("-[r{}]->", i - 1)),
                }
            }
            match &node.label {
                Some(l) => q.push_str(&format!("(n{i}:{l})")),
                None => q.push_str(&format!("(n{i})")),
            }
        }
        q.push_str(" RETURN ");
        let proj: Vec<String> = (0..self.nodes.len()).map(|i| format!("n{i}.id")).collect();
        q.push_str(&proj.join(", "));
        q
    }
}

// =====================================================================
// Free-function façade (one-shot convenience; benches should reuse Oracle)
// =====================================================================

/// k-hop expansion from `sources`.
///
/// **Normative semantics** (S3 implements exactly this):
/// - Result is the set of `(src, dst, depth)` triples where `dst` is
///   reachable from `src` in at most `k` hops over edges admitted by
///   `dir`/`etype`, and `depth` is the **shortest** such hop count.
/// - **Node-distinct per source**: each `(src, dst)` pair appears exactly
///   once, at its minimal depth — never once per path.
/// - **Depth 0**: every source that exists in the node set contributes its
///   own `(s, s, 0)` row (mirrors KQL `graph-traverse`, whose anchor row has
///   `depth == 0`). Source ids absent from the node set contribute nothing.
/// - **Cycle-safe**: cycles cannot loop (visited set per source); a 5-ring
///   from a member yields depths 0..=4 with `k ≥ 4`, never more.
/// - `dir = Back` follows edges in reverse; `Both` treats edges as
///   undirected. `etype` filters edges by exact type equality before
///   traversal.
pub fn k_hop(
    g: &TestGraph,
    sources: &[&str],
    k: usize,
    dir: Direction,
    etype: Option<&str>,
) -> BTreeSet<(String, String, usize)> {
    Oracle::new(g).k_hop(sources, k, dir, etype)
}

/// Var-length expansion `-[*min..max]->` from `sources`.
///
/// **Normative semantics**: identical to [`k_hop`] with `k = max`, keeping
/// only triples with `depth ≥ min`. In particular this is **shortest-depth /
/// node-distinct (walk) semantics**, *not* Cypher's edge-distinct trail
/// semantics: a `dst` whose shortest distance is 1 is **excluded** by
/// `min = 2` even if some longer redundant path of length 2 also reaches it.
/// This is the cheap vectorizable contract S3 targets; it is deliberately
/// simpler than openCypher.
pub fn var_length(
    g: &TestGraph,
    sources: &[&str],
    min: usize,
    max: usize,
    dir: Direction,
    etype: Option<&str>,
) -> BTreeSet<(String, String, usize)> {
    Oracle::new(g).var_length(sources, min, max, dir, etype)
}

/// Unweighted shortest path length (BFS hop count).
///
/// **Normative semantics**:
/// - `Some(d)` where `d` is the minimal number of admitted edges from `src`
///   to `dst`; `Some(0)` when `src == dst` and the node exists (mirrors KQL
///   `graph-shortest-path`, which reports `depth 0` for self-paths).
/// - `None` when unreachable, or when either endpoint id is absent from the
///   node set.
/// - `dir`/`etype` as in [`k_hop`].
pub fn shortest_path_len(
    g: &TestGraph,
    src: &str,
    dst: &str,
    dir: Direction,
    etype: Option<&str>,
) -> Option<usize> {
    Oracle::new(g).shortest_path_len(src, dst, dir, etype)
}

/// Evaluate a forward MATCH chain, returning the set of bound node-id tuples
/// `[n0, n1, …, nK]`.
///
/// **Normative semantics** — exactly what the existing Cypher subset compiles
/// to (a (2K+1)-way SQL inner join, see `kyma-kql`'s `graph-match` lowering):
/// - **Homomorphism semantics**: node and edge variables may bind to the same
///   node/edge more than once within a tuple (`a→b→a` over a 2-cycle yields
///   `[a, b, a]`). There is **no** node- or edge-distinctness constraint.
/// - **Label filter** `(:L)`: the node's *primary label* — the single string
///   the testkit ingests into the `labels` column — must equal `L` exactly.
///   (The engine lowers `:L` to `labels == 'L'`; multi-label nodes are out of
///   scope for the v1 subset.)
/// - **Type filter** `[:T]`: the edge's `etype` must equal `T` exactly.
/// - **Dangling edges** (endpoint missing from the node set) never match —
///   the engine inner-joins the node table on both endpoints.
/// - **Set comparison**: the engine returns a *bag* (duplicate edges produce
///   duplicate rows); the differential harness compares distinct tuples, so
///   the oracle returns a set.
pub fn match_chain(g: &TestGraph, pattern: &ChainPattern) -> BTreeSet<Vec<String>> {
    Oracle::new(g).match_chain(pattern)
}
