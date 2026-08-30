//! Pure oracle tests — no engine, no docker.
//!
//! Hand-computed expectations on structured topologies + property tests on
//! random small graphs. These pin the oracle's semantics (the S3 spec) before
//! anything is differentially compared against it.

use std::collections::BTreeSet;

use pensieve_graph_testkit::model::TestGraph;
use pensieve_graph_testkit::oracle::{
    k_hop, match_chain, shortest_path_len, var_length, ChainPattern, Direction,
};
use pensieve_graph_testkit::synth;

fn triples(
    items: &[(&str, &str, usize)],
) -> BTreeSet<(String, String, usize)> {
    items
        .iter()
        .map(|(a, b, d)| (a.to_string(), b.to_string(), *d))
        .collect()
}

// ====================================================================
// k_hop on a grid — hand-computed
// ====================================================================

#[test]
fn grid_k_hop_forward_from_corner() {
    let g = synth::grid(4, 4);
    // From g0_0, forward, k=2, any type. Grid edges go right (LINKS) and
    // down (CALLS), so forward BFS reaches the lower-right wedge.
    let got = k_hop(&g, &["g0_0"], 2, Direction::Fwd, None);
    let want = triples(&[
        ("g0_0", "g0_0", 0),
        ("g0_0", "g0_1", 1),
        ("g0_0", "g1_0", 1),
        ("g0_0", "g0_2", 2),
        ("g0_0", "g1_1", 2),
        ("g0_0", "g2_0", 2),
    ]);
    assert_eq!(got, want);
}

#[test]
fn grid_k_hop_etype_filter_restricts_to_one_axis() {
    let g = synth::grid(4, 4);
    // Only LINKS (horizontal) edges: BFS walks along row 0 only.
    let got = k_hop(&g, &["g0_0"], 3, Direction::Fwd, Some("LINKS"));
    let want = triples(&[
        ("g0_0", "g0_0", 0),
        ("g0_0", "g0_1", 1),
        ("g0_0", "g0_2", 2),
        ("g0_0", "g0_3", 3),
    ]);
    assert_eq!(got, want);
}

#[test]
fn grid_k_hop_backward_mirrors_forward() {
    let g = synth::grid(4, 4);
    // Backward from the far corner reaches the same wedge, mirrored.
    let got = k_hop(&g, &["g3_3"], 2, Direction::Back, None);
    let want = triples(&[
        ("g3_3", "g3_3", 0),
        ("g3_3", "g3_2", 1),
        ("g3_3", "g2_3", 1),
        ("g3_3", "g3_1", 2),
        ("g3_3", "g2_2", 2),
        ("g3_3", "g1_3", 2),
    ]);
    assert_eq!(got, want);
}

#[test]
fn grid_k_hop_unknown_source_contributes_nothing() {
    let g = synth::grid(2, 2);
    let got = k_hop(&g, &["nope"], 3, Direction::Fwd, None);
    assert!(got.is_empty());
}

// ====================================================================
// k_hop / shortest_path on cycles — cycle safety + shortest depths
// ====================================================================

#[test]
fn ring_k_hop_is_cycle_safe_and_depths_are_shortest() {
    // Single directed 5-ring: c0_0 → c0_1 → … → c0_4 → c0_0.
    let g = synth::cyclic(&[5], 0, 1);
    let got = k_hop(&g, &["c0_0"], 10, Direction::Fwd, None);
    // Even with k=10, each node appears exactly once at its shortest depth.
    let want = triples(&[
        ("c0_0", "c0_0", 0),
        ("c0_0", "c0_1", 1),
        ("c0_0", "c0_2", 2),
        ("c0_0", "c0_3", 3),
        ("c0_0", "c0_4", 4),
    ]);
    assert_eq!(got, want);
}

#[test]
fn ring_both_direction_halves_distances() {
    let g = synth::cyclic(&[6], 0, 1);
    // Undirected on a 6-ring: c0_5 is 1 hop away (backwards), c0_3 is 3.
    assert_eq!(
        shortest_path_len(&g, "c0_0", "c0_5", Direction::Both, None),
        Some(1)
    );
    assert_eq!(
        shortest_path_len(&g, "c0_0", "c0_5", Direction::Fwd, None),
        Some(5)
    );
    assert_eq!(
        shortest_path_len(&g, "c0_0", "c0_3", Direction::Both, None),
        Some(3)
    );
}

#[test]
fn shortest_path_self_and_unreachable() {
    let g = synth::cyclic(&[4, 4], 0, 1); // two disjoint rings, no cross links
    assert_eq!(
        shortest_path_len(&g, "c0_2", "c0_2", Direction::Fwd, None),
        Some(0),
        "self-path is depth 0 (engine parity: graph-shortest-path api→api = 0)"
    );
    assert_eq!(
        shortest_path_len(&g, "c0_0", "c1_0", Direction::Both, None),
        None,
        "disjoint rings are unreachable"
    );
    assert_eq!(
        shortest_path_len(&g, "c0_0", "missing", Direction::Fwd, None),
        None
    );
}

// ====================================================================
// var_length
// ====================================================================

#[test]
fn var_length_is_k_hop_with_min_bound() {
    let g = synth::grid(4, 4);
    let got = var_length(&g, &["g0_0"], 2, 3, Direction::Fwd, None);
    let all = k_hop(&g, &["g0_0"], 3, Direction::Fwd, None);
    let want: BTreeSet<_> = all.into_iter().filter(|(_, _, d)| *d >= 2).collect();
    assert_eq!(got, want);
    assert!(got.iter().all(|(_, _, d)| (2..=3).contains(d)));
    // The depth-0 self row and depth-1 neighbors are excluded by min=2.
    assert!(!got.iter().any(|(_, dst, _)| dst == "g0_0" || dst == "g0_1"));
}

// ====================================================================
// match_chain on a tiny labeled graph — hand enumeration
// ====================================================================

/// a(Hub) -LINKS-> b(N) -CALLS-> c(N); b -LINKS-> a (back edge ⇒ 2-cycle a/b);
/// d(N) dangling-edge target that is NOT in the node set.
fn tiny() -> TestGraph {
    let mut g = TestGraph::new();
    g.add_node("a", "Hub");
    g.add_node("b", "N");
    g.add_node("c", "N");
    g.add_edge("a", "b", "LINKS");
    g.add_edge("b", "c", "CALLS");
    g.add_edge("b", "a", "LINKS");
    g.add_edge("c", "ghost", "LINKS"); // dangling: `ghost` has no node row
    g
}

fn tuples(items: &[&[&str]]) -> BTreeSet<Vec<String>> {
    items
        .iter()
        .map(|t| t.iter().map(|s| s.to_string()).collect())
        .collect()
}

#[test]
fn match_chain_single_hop_untyped() {
    let g = tiny();
    let got = match_chain(&g, &ChainPattern::start(None).hop(None, None));
    // Dangling edge c→ghost must NOT match (inner-join semantics).
    let want = tuples(&[&["a", "b"], &["b", "c"], &["b", "a"]]);
    assert_eq!(got, want);
}

#[test]
fn match_chain_type_and_label_filters() {
    let g = tiny();
    let got = match_chain(&g, &ChainPattern::start(Some("Hub")).hop(Some("LINKS"), None));
    assert_eq!(got, tuples(&[&["a", "b"]]));

    let got = match_chain(&g, &ChainPattern::start(None).hop(Some("CALLS"), Some("N")));
    assert_eq!(got, tuples(&[&["b", "c"]]));
}

#[test]
fn match_chain_two_hop_homomorphism_allows_repeated_nodes() {
    let g = tiny();
    let got = match_chain(&g, &ChainPattern::start(None).hop(None, None).hop(None, None));
    // a→b→c, a→b→a (revisits a — allowed!), b→a→b (revisits b — allowed!)
    let want = tuples(&[&["a", "b", "c"], &["a", "b", "a"], &["b", "a", "b"]]);
    assert_eq!(got, want);
}

#[test]
fn match_chain_middle_label_filter() {
    let g = tiny();
    let got = match_chain(
        &g,
        &ChainPattern::start(None).hop(None, Some("Hub")).hop(None, None),
    );
    // middle node must be a (the only Hub): b→a→b only.
    assert_eq!(got, tuples(&[&["b", "a", "b"]]));
}

// ====================================================================
// Property tests on random small graphs
// ====================================================================

use proptest::prelude::*;

/// Random graph: `n` nodes (ids x0..x{n-1}, labels alternating N/Hub),
/// edges from an arbitrary pair list, types alternating LINKS/CALLS.
fn arb_graph() -> impl Strategy<Value = TestGraph> {
    (2usize..12, proptest::collection::vec((0usize..12, 0usize..12), 0..40)).prop_map(
        |(n, pairs)| {
            let mut g = TestGraph::new();
            for i in 0..n {
                g.add_node(format!("x{i}"), if i % 3 == 0 { "Hub" } else { "N" });
            }
            for (k, (a, b)) in pairs.into_iter().enumerate() {
                let (a, b) = (a % n, b % n);
                g.add_edge(
                    format!("x{a}"),
                    format!("x{b}"),
                    if k % 2 == 0 { "LINKS" } else { "CALLS" },
                );
            }
            g
        },
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// k_hop(k=1) over all sources == depth-0 self rows + direct distinct
    /// adjacency (minus self-loops, which BFS never reports at depth 1
    /// because the source is already visited at depth 0).
    #[test]
    fn k_hop_1_equals_direct_adjacency(g in arb_graph()) {
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        let got = k_hop(&g, &ids, 1, Direction::Fwd, None);
        let mut want: BTreeSet<(String, String, usize)> = ids
            .iter()
            .map(|s| (s.to_string(), s.to_string(), 0))
            .collect();
        for e in &g.edges {
            if e.src != e.dst {
                want.insert((e.src.clone(), e.dst.clone(), 1));
            }
        }
        prop_assert_eq!(got, want);
    }

    /// Triangle inequality: d(a,c) ≤ d(a,b) + d(b,c) whenever both legs exist.
    #[test]
    fn shortest_path_triangle_inequality(g in arb_graph(), a in 0usize..12, b in 0usize..12, c in 0usize..12) {
        let n = g.node_count();
        let (a, b, c) = (format!("x{}", a % n), format!("x{}", b % n), format!("x{}", c % n));
        let dab = shortest_path_len(&g, &a, &b, Direction::Fwd, None);
        let dbc = shortest_path_len(&g, &b, &c, Direction::Fwd, None);
        let dac = shortest_path_len(&g, &a, &c, Direction::Fwd, None);
        if let (Some(x), Some(y)) = (dab, dbc) {
            let dac = dac.expect("a→b and b→c reachable ⇒ a→c reachable");
            prop_assert!(dac <= x + y, "d(a,c)={dac} > d(a,b)+d(b,c)={}", x + y);
        }
        prop_assert_eq!(shortest_path_len(&g, &a, &a, Direction::Fwd, None), Some(0));
    }

    /// match_chain 1-hop untyped/unlabeled == distinct (src,dst) edge pairs.
    #[test]
    fn match_chain_1_hop_equals_edge_set(g in arb_graph()) {
        let got = match_chain(&g, &ChainPattern::start(None).hop(None, None));
        let want: BTreeSet<Vec<String>> = g
            .edges
            .iter()
            .map(|e| vec![e.src.clone(), e.dst.clone()])
            .collect();
        prop_assert_eq!(got, want);
    }

    /// k_hop depths are internally consistent with shortest_path_len.
    #[test]
    fn k_hop_depth_equals_shortest_path(g in arb_graph()) {
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        let hops = k_hop(&g, &ids, 4, Direction::Fwd, None);
        for (s, d, depth) in &hops {
            let sp = shortest_path_len(&g, s, d, Direction::Fwd, None);
            prop_assert_eq!(sp, Some(*depth), "k_hop depth vs BFS for {} → {}", s, d);
        }
    }
}
