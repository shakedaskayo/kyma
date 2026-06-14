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
