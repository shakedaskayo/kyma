//! Differential validation of [`CsrGraph`] against the petgraph reference
//! oracle (`kyma_graph_testkit::oracle`). The oracle's doc comments are the
//! normative spec; this test asserts CSR traversal matches it set-for-set over
//! every generator topology — including cyclic — and a proptest sweep.

use kyma_graph_testkit::model::TestGraph;
use kyma_graph_testkit::oracle::{Direction as ODir, Oracle};
use kyma_graph_testkit::synth;
use kyma_graph_topo::{CsrGraph, Direction};

fn csr_of(g: &TestGraph) -> CsrGraph {
    CsrGraph::build(
        g.nodes.iter().map(|n| n.id.as_str()),
        g.edges
            .iter()
            .map(|e| (e.src.as_str(), e.dst.as_str(), e.etype.as_str())),
    )
}

const DIRS: [(Direction, ODir); 3] = [
    (Direction::Fwd, ODir::Fwd),
    (Direction::Back, ODir::Back),
    (Direction::Both, ODir::Both),
];

/// Compare every traversal primitive over a representative sweep of
/// (source, k, direction, etype) parameters.
fn assert_matches_oracle(g: &TestGraph, label: &str) {
    let csr = csr_of(g);
    let oracle = Oracle::new(g);

    // Sources: a deterministic spread across the id space, plus an unknown id.
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let step = (ids.len() / 7).max(1);
    let mut sources: Vec<&str> = ids.iter().step_by(step).copied().collect();
    sources.push("definitely-not-a-node");

    // etype filters: none, each distinct type, and an absent type.
    let mut etypes: Vec<Option<&str>> = vec![None, Some("definitely-not-a-type")];
    for t in g.edge_types() {
        etypes.push(Some(t));
    }

    for &src in &sources {
        for k in [0usize, 1, 2, 3, 5] {
            for &(cd, od) in &DIRS {
                for &et in &etypes {
                    let got = csr.k_hop(&[src], k, cd, et);
                    let want = oracle.k_hop(&[src], k, od, et);
                    assert_eq!(
                        got, want,
                        "{label}: k_hop src={src} k={k} dir={cd:?} et={et:?}"
                    );
                }
            }
        }
    }

    // Multi-source k_hop (union semantics) on the full source list.
    for k in [1usize, 3] {
        for &(cd, od) in &DIRS {
            let got = csr.k_hop(&sources, k, cd, None);
            let want = oracle.k_hop(&sources, k, od, None);
            assert_eq!(got, want, "{label}: multi-source k_hop k={k} dir={cd:?}");
        }
    }

    // var_length windows.
    for (min, max) in [(0usize, 2usize), (2, 4), (1, 3)] {
        for &(cd, od) in &DIRS {
            let got = csr.var_length(&sources, min, max, cd, None);
            let want = oracle.var_length(&sources, min, max, od, None);
            assert_eq!(got, want, "{label}: var_length [{min}..{max}] dir={cd:?}");
        }
    }

    // shortest_path between a grid of endpoint pairs.
    let endpoints: Vec<&str> = ids.iter().step_by(step).copied().collect();
    for &a in &endpoints {
        for &b in &endpoints {
            for &(cd, od) in &DIRS {
                let got = csr.shortest_path_len(a, b, cd, None);
                let want = oracle.shortest_path_len(a, b, od, None);
                assert_eq!(got, want, "{label}: shortest {a}->{b} dir={cd:?}");
            }
        }
    }
}

#[test]
fn matches_oracle_on_power_law() {
    assert_matches_oracle(&synth::power_law(400, 4, 0xC57A), "power_law");
}

#[test]
fn matches_oracle_on_grid() {
    assert_matches_oracle(&synth::grid(20, 25), "grid");
}

#[test]
fn matches_oracle_on_hub_and_spoke() {
    assert_matches_oracle(&synth::hub_and_spoke(8, 40), "hub_and_spoke");
}

#[test]
fn matches_oracle_on_cyclic() {
    // Multiple rings + cross-links: cycles, back-edges, and multiple types.
    assert_matches_oracle(&synth::cyclic(&[30, 25, 40, 15], 50, 7), "cyclic");
}

/// S3.4 gate: forward-push PPR must match the exact power-iteration PPR vector
/// (L1 < 1e-3). The reference builds the undirected ("Both") transition matrix
/// the same way `CsrGraph::step(Both)` enumerates neighbors — each directed
/// edge `(s,d)` contributes `s→d` and `d→s` — so the two agree by construction
/// when the approximation is tight.
#[test]
fn ppr_matches_power_iteration() {
    let g = synth::grid(5, 5); // 25 nodes, connected, no sinks under Both
    let csr = csr_of(&g);
    let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
    let idx: std::collections::HashMap<&str, usize> =
        ids.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let n = ids.len();

    // Both-adjacency mirror of step(Both).
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &g.edges {
        if let (Some(&s), Some(&d)) = (idx.get(e.src.as_str()), idx.get(e.dst.as_str())) {
            adj[s].push(d);
            adj[d].push(s);
        }
    }

    let alpha = 0.15;
    let seed = ids[0];
    let seed_i = idx[seed];

    // Exact PPR via power iteration: pi = alpha·s + (1-alpha)·Pᵀ·pi.
    let mut s = vec![0.0f64; n];
    s[seed_i] = 1.0;
    let mut pi = s.clone();
    for _ in 0..5000 {
        let mut np = vec![0.0f64; n];
        for u in 0..n {
            if adj[u].is_empty() {
                continue;
            }
            let share = (1.0 - alpha) * pi[u] / adj[u].len() as f64;
            for &v in &adj[u] {
                np[v] += share;
            }
        }
        for v in 0..n {
            np[v] += alpha * s[v];
        }
        let delta: f64 = (0..n).map(|i| (np[i] - pi[i]).abs()).sum();
        pi = np;
        if delta < 1e-15 {
            break;
        }
    }

    // Forward-push approximation with a tight epsilon.
    let approx = csr.personalized_pagerank(&[seed], alpha, 1e-9, Direction::Both);
    let mut p = vec![0.0f64; n];
    for (id, m) in &approx {
        p[idx[id.as_str()]] = *m;
    }
    let l1: f64 = (0..n).map(|i| (p[i] - pi[i]).abs()).sum();
    assert!(
        l1 < 1e-3,
        "PPR forward-push vs power-iteration L1={l1} (want <1e-3)"
    );
    // The restart seed carries the most mass.
    assert_eq!(approx.first().map(|(id, _)| id.as_str()), Some(seed));
}

#[test]
fn connected_components_splits_disjoint_and_joins_connected() {
    // Two disjoint triangles → two weakly-connected components.
    let mut g = TestGraph::new();
    for id in ["x0", "x1", "x2", "y0", "y1", "y2"] {
        g.add_node(id, "N");
    }
    for (s, d) in [("x0", "x1"), ("x1", "x2"), ("x2", "x0")] {
        g.add_edge(s, d, "E");
    }
    for (s, d) in [("y0", "y1"), ("y1", "y2"), ("y2", "y0")] {
        g.add_edge(s, d, "E");
    }
    let csr = csr_of(&g);
    let comp: std::collections::HashMap<String, u32> = csr
        .connected_components(Direction::Both)
        .into_iter()
        .collect();
    let distinct: std::collections::BTreeSet<u32> = comp.values().copied().collect();
    assert_eq!(distinct.len(), 2, "two disjoint triangles → 2 components");
    assert_eq!(comp["x0"], comp["x1"]);
    assert_eq!(comp["x0"], comp["x2"]);
    assert_eq!(comp["y0"], comp["y2"]);
    assert_ne!(
        comp["x0"], comp["y0"],
        "the triangles are separate components"
    );

    // Add a bridge → one component.
    g.add_edge("x0", "y0", "BRIDGE");
    let csr2 = csr_of(&g);
    let one: std::collections::BTreeSet<u32> = csr2
        .connected_components(Direction::Both)
        .into_iter()
        .map(|(_, c)| c)
        .collect();
    assert_eq!(one.len(), 1, "a bridge joins them into one component");
}

#[test]
fn ppr_unknown_seed_is_empty() {
    let g = synth::grid(3, 3);
    let csr = csr_of(&g);
    assert!(csr
        .personalized_pagerank(&["nope"], 0.15, 1e-6, Direction::Both)
        .is_empty());
}

/// Two 6-cliques joined by a single bridge — the planted partition that LPA
/// fails. Modularity-based Louvain must (1) recover the two communities, (2)
/// score a high modularity well above the all-singletons baseline, and (3) be
/// deterministic.
#[test]
fn louvain_recovers_planted_partition() {
    let mut g = TestGraph::new();
    let mut clique = |g: &mut TestGraph, prefix: &str| -> Vec<String> {
        let ids: Vec<String> = (0..6).map(|i| format!("{prefix}{i}")).collect();
        for id in &ids {
            g.add_node(id.clone(), "N");
        }
        for i in 0..ids.len() {
            for j in (i + 1)..ids.len() {
                g.add_edge(ids[i].clone(), ids[j].clone(), "E");
            }
        }
        ids
    };
    let a = clique(&mut g, "a");
    let b = clique(&mut g, "b");
    g.add_edge(a[0].clone(), b[0].clone(), "BRIDGE");

    let csr = csr_of(&g);
    let parts = csr.detect_communities(Direction::Both, 50);
    let comm: std::collections::HashMap<&str, u32> =
        parts.iter().map(|(id, c)| (id.as_str(), *c)).collect();

    let ca = comm[a[0].as_str()];
    let cb = comm[b[0].as_str()];
    assert_ne!(ca, cb, "the two cliques are distinct communities");
    for id in &a {
        assert_eq!(comm[id.as_str()], ca, "clique A is one community");
    }
    for id in &b {
        assert_eq!(comm[id.as_str()], cb, "clique B is one community");
    }

    // Modularity (node-index order) beats the all-singletons baseline and is high.
    let labels: Vec<u32> = parts.iter().map(|(_, c)| *c).collect();
    let q = csr.modularity(&labels, Direction::Both);
    let singletons: Vec<u32> = (0..labels.len() as u32).collect();
    let q0 = csr.modularity(&singletons, Direction::Both);
    assert!(q > q0, "partition modularity {q} must beat singletons {q0}");
    assert!(
        q > 0.3,
        "two-clique partition should score high modularity, got {q}"
    );

    // Deterministic.
    assert_eq!(parts, csr.detect_communities(Direction::Both, 50));
}

mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// A random small directed multigraph: `n` nodes, random typed edges
    /// (including self-loops and parallel edges).
    fn arb_graph() -> impl Strategy<Value = TestGraph> {
        (2usize..14)
            .prop_flat_map(|n| {
                let edges = prop::collection::vec((0..n, 0..n, 0u8..3), 0..(n * 3));
                (Just(n), edges)
            })
            .prop_map(|(n, edges)| {
                let mut g = TestGraph::new();
                for i in 0..n {
                    g.add_node(format!("n{i}"), "N");
                }
                for (s, d, t) in edges {
                    g.add_edge(format!("n{s}"), format!("n{d}"), format!("T{t}"));
                }
                g
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(300))]

        #[test]
        fn k_hop_and_shortest_match_oracle(g in arb_graph()) {
            let csr = csr_of(&g);
            let oracle = Oracle::new(&g);
            let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
            for &src in &ids {
                for k in [0usize, 1, 2, 4] {
                    for &(cd, od) in &DIRS {
                        prop_assert_eq!(
                            csr.k_hop(&[src], k, cd, None),
                            oracle.k_hop(&[src], k, od, None)
                        );
                    }
                }
            }
            for &a in &ids {
                for &b in &ids {
                    prop_assert_eq!(
                        csr.shortest_path_len(a, b, Direction::Fwd, None),
                        oracle.shortest_path_len(a, b, ODir::Fwd, None)
                    );
                }
            }
        }
    }
}
