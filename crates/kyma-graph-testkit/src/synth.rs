//! Seeded, deterministic topology generators.
//!
//! Every generator is a pure function of its arguments: the same call
//! produces a byte-identical [`TestGraph`] (node order, edge order, labels,
//! types) on every platform — `fastrand::Rng::with_seed` is the only
//! randomness source. The differential gate relies on this to reproduce a
//! mismatch from nothing but the generator name + parameters + seed.
//!
//! Labeling/typing conventions (so Cypher `:Label` / `[:TYPE]` filters are
//! exercisable on every topology):
//! - node labels: `"N"` for ordinary nodes, `"Hub"` for designated hubs
//!   (power-law seed nodes, hub-and-spoke hubs, ring entry nodes).
//! - edge types: `"LINKS"` and `"CALLS"`, deterministically assigned per
//!   generator (documented on each function).

use crate::model::TestGraph;

/// Barabási–Albert preferential attachment.
///
/// - `n_nodes` total nodes, ids `n0..n{n-1}` in creation order.
/// - `avg_degree` target average *total* degree; each new node attaches with
///   `m = max(1, avg_degree / 2)` edges pointing **from the new node to
///   existing nodes** (so the digraph is acyclic and out-degree ≤ m, while
///   in-degree is power-law distributed).
/// - Attachment targets are sampled proportionally to current degree via the
///   classic repeated-endpoints list; duplicate targets within one node's
///   batch are re-rolled (bounded), so a node's `m` out-edges are distinct.
/// - The first `m + 1` seed nodes get label `"Hub"` (they end up highest
///   degree); everything else `"N"`.
/// - Edge `i` (creation order) gets type `"CALLS"` when `i % 4 == 0`, else
///   `"LINKS"`.
pub fn power_law(n_nodes: usize, avg_degree: usize, seed: u64) -> TestGraph {
    let m = (avg_degree / 2).max(1);
    let mut g = TestGraph::new();
    let mut rng = fastrand::Rng::with_seed(seed);

    let n_seed = (m + 1).min(n_nodes);
    for i in 0..n_nodes {
        let label = if i < n_seed { "Hub" } else { "N" };
        g.add_node(format!("n{i}"), label);
    }

    // Repeated-endpoints list: node index appears once per incident edge
    // (+1 baseline for seed nodes so the very first attachments have targets).
    let mut endpoints: Vec<usize> = (0..n_seed).collect();

    let mut edge_ordinal = 0usize;
    let mut push_edge = |g: &mut TestGraph, src: usize, dst: usize| {
        let etype = if edge_ordinal % 4 == 0 { "CALLS" } else { "LINKS" };
        g.add_edge(format!("n{src}"), format!("n{dst}"), etype);
        edge_ordinal += 1;
    };

    // Wire the seed clique-ish chain so seed nodes are connected.
    for i in 1..n_seed {
        push_edge(&mut g, i, i - 1);
        endpoints.push(i);
        endpoints.push(i - 1);
    }

    for new in n_seed..n_nodes {
        let mut targets: Vec<usize> = Vec::with_capacity(m);
        let mut attempts = 0usize;
        while targets.len() < m && attempts < 32 * m {
            attempts += 1;
            let t = endpoints[rng.usize(..endpoints.len())];
            if t != new && !targets.contains(&t) {
                targets.push(t);
            }
        }
        for &t in &targets {
            push_edge(&mut g, new, t);
            endpoints.push(new);
            endpoints.push(t);
        }
    }
    g
}

/// `rows × cols` directed grid lattice.
///
/// - Node ids `g{r}_{c}`, all labeled `"N"`.
/// - Horizontal edges `g{r}_{c} → g{r}_{c+1}` typed `"LINKS"`, vertical edges
///   `g{r}_{c} → g{r+1}_{c}` typed `"CALLS"`. Row-major emission order:
///   for each node, right edge then down edge.
/// - Fully deterministic — no RNG involved.
pub fn grid(rows: usize, cols: usize) -> TestGraph {
    let mut g = TestGraph::new();
    for r in 0..rows {
        for c in 0..cols {
            g.add_node(format!("g{r}_{c}"), "N");
        }
    }
    for r in 0..rows {
        for c in 0..cols {
            if c + 1 < cols {
                g.add_edge(format!("g{r}_{c}"), format!("g{r}_{}", c + 1), "LINKS");
            }
            if r + 1 < rows {
                g.add_edge(format!("g{r}_{c}"), format!("g{}_{c}", r + 1), "CALLS");
            }
        }
    }
    g
}

/// `n_hubs` hubs, each fanning out to `spokes_per_hub` leaf spokes.
///
/// - Hub ids `h{i}` labeled `"Hub"`; spoke ids `h{i}_s{j}` labeled `"N"`.
/// - Hub → spoke edges typed `"LINKS"`; consecutive hubs are chained
///   `h{i} → h{i+1}` typed `"CALLS"` so the hubs form a backbone path.
/// - Fully deterministic — no RNG involved.
pub fn hub_and_spoke(n_hubs: usize, spokes_per_hub: usize) -> TestGraph {
    let mut g = TestGraph::new();
    for i in 0..n_hubs {
        g.add_node(format!("h{i}"), "Hub");
        for j in 0..spokes_per_hub {
            g.add_node(format!("h{i}_s{j}"), "N");
        }
    }
    for i in 0..n_hubs {
        if i + 1 < n_hubs {
            g.add_edge(format!("h{i}"), format!("h{}", i + 1), "CALLS");
        }
        for j in 0..spokes_per_hub {
            g.add_edge(format!("h{i}"), format!("h{i}_s{j}"), "LINKS");
        }
    }
    g
}

/// Disjoint directed rings plus seeded random cross-links between rings.
///
/// - Ring `k` of size `ring_sizes[k]` has node ids `c{k}_{i}`; node `c{k}_0`
///   is labeled `"Hub"`, the rest `"N"`. Ring edges `c{k}_{i} → c{k}_{i+1}`
///   (wrapping) are typed `"LINKS"`.
/// - `cross_links` extra edges connect a random node of one ring to a random
///   node of a *different* ring (seeded; skipped when fewer than 2 rings),
///   typed `"CALLS"`.
/// - Cycles guarantee the oracle's visited-set / cycle-safety paths are
///   exercised.
pub fn cyclic(ring_sizes: &[usize], cross_links: usize, seed: u64) -> TestGraph {
    let mut g = TestGraph::new();
    let mut rng = fastrand::Rng::with_seed(seed);

    for (k, &sz) in ring_sizes.iter().enumerate() {
        for i in 0..sz {
            let label = if i == 0 { "Hub" } else { "N" };
            g.add_node(format!("c{k}_{i}"), label);
        }
    }
    for (k, &sz) in ring_sizes.iter().enumerate() {
        for i in 0..sz {
            g.add_edge(format!("c{k}_{i}"), format!("c{k}_{}", (i + 1) % sz), "LINKS");
        }
    }
    let n_rings = ring_sizes.len();
    if n_rings >= 2 {
        for _ in 0..cross_links {
            let from_ring = rng.usize(..n_rings);
            let mut to_ring = rng.usize(..n_rings);
            while to_ring == from_ring {
                to_ring = rng.usize(..n_rings);
            }
            let fi = rng.usize(..ring_sizes[from_ring]);
            let ti = rng.usize(..ring_sizes[to_ring]);
            g.add_edge(format!("c{from_ring}_{fi}"), format!("c{to_ring}_{ti}"), "CALLS");
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_law_same_seed_identical_edge_list() {
        let a = power_law(500, 4, 1234);
        let b = power_law(500, 4, 1234);
        assert_eq!(a, b, "same seed must produce an identical graph");
        let c = power_law(500, 4, 1235);
        assert_ne!(a.edges, c.edges, "different seed should differ");
    }

    #[test]
    fn cyclic_same_seed_identical_edge_list() {
        let a = cyclic(&[10, 20, 30], 15, 99);
        let b = cyclic(&[10, 20, 30], 15, 99);
        assert_eq!(a, b);
    }

    #[test]
    fn generators_emit_both_edge_types_and_both_labels() {
        for g in [
            power_law(200, 4, 7),
            grid(5, 5),
            hub_and_spoke(4, 6),
            cyclic(&[8, 9], 6, 3),
        ] {
            let types = g.edge_types();
            assert!(types.contains(&"LINKS"), "missing LINKS: {types:?}");
            // grid is the only one guaranteed CALLS via vertical edges too.
            assert!(types.contains(&"CALLS"), "missing CALLS: {types:?}");
        }
        assert!(power_law(200, 4, 7).nodes.iter().any(|n| n.primary_label() == Some("Hub")));
        assert!(hub_and_spoke(4, 6).nodes.iter().any(|n| n.primary_label() == Some("Hub")));
        assert!(cyclic(&[8, 9], 6, 3).nodes.iter().any(|n| n.primary_label() == Some("Hub")));
    }

    #[test]
    fn grid_counts() {
        let g = grid(3, 4);
        assert_eq!(g.node_count(), 12);
        // horizontal: 3 rows * 3, vertical: 2 rows * 4
        assert_eq!(g.edge_count(), 9 + 8);
    }
}
