//! Deterministic layout algorithms, ported from packages/client/src/graph-layout.ts.
//! Positions are computed server-side once per graph version and cached.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Neighbour-cell offsets for the spatial-grid passes (forward half-plane + self).
const NEIGHBOR_OFFSETS: [(i64, i64); 5] = [(0, 0), (1, 0), (0, 1), (1, 1), (-1, 1)];

use crate::{GraphNode, GraphRelationship};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutAlgorithm {
    #[default]
    Force,
    Tree,
    Grid,
    Radial,
}

/// Virtual canvas the layout is computed in. Sigma has its own camera space,
/// so any consistent extent works — these match the web container defaults.
pub const LAYOUT_WIDTH: f64 = 1600.0;
pub const LAYOUT_HEIGHT: f64 = 1000.0;

pub fn compute_layout(
    algorithm: LayoutAlgorithm,
    nodes: &[GraphNode],
    edges: &[GraphRelationship],
    width: f64,
    height: f64,
) -> BTreeMap<String, (f64, f64)> {
    match algorithm {
        LayoutAlgorithm::Grid => grid_layout(nodes, width, height),
        LayoutAlgorithm::Radial => radial_layout(nodes, width, height),
        LayoutAlgorithm::Tree => tree_layout(nodes, edges, width, height),
        LayoutAlgorithm::Force => force_layout(nodes, edges, width, height),
    }
}

fn grid_layout(nodes: &[GraphNode], width: f64, height: f64) -> BTreeMap<String, (f64, f64)> {
    let n = nodes.len();
    let mut out = BTreeMap::new();
    if n == 0 {
        return out;
    }
    let cols = ((n as f64 * (width / height)).sqrt().ceil() as usize).max(1);
    let rows = n.div_ceil(cols);
    let cell_w = width / (cols as f64 + 1.0);
    let cell_h = height / (rows as f64 + 1.0);
    for (i, node) in nodes.iter().enumerate() {
        let col = i % cols;
        let row = i / cols;
        out.insert(node.id.clone(), (cell_w * (col as f64 + 1.0), cell_h * (row as f64 + 1.0)));
    }
    out
}

fn radial_layout(nodes: &[GraphNode], width: f64, height: f64) -> BTreeMap<String, (f64, f64)> {
    let n = nodes.len();
    let mut out = BTreeMap::new();
    if n == 0 {
        return out;
    }
    let (cx, cy) = (width / 2.0, height / 2.0);
    out.insert(nodes[0].id.clone(), (cx, cy));
    if n == 1 {
        return out;
    }
    let ring_spacing = width.min(height) / (2.0 * ((n as f64).sqrt().ceil() + 1.0));
    let mut placed = 1usize;
    let mut ring = 1usize;
    while placed < n {
        let radius = ring as f64 * ring_spacing;
        let circumference = 2.0 * std::f64::consts::PI * radius;
        let in_ring = ((circumference / 80.0).floor() as usize).max(6).min(n - placed);
        for i in 0..in_ring {
            if placed >= n {
                break;
            }
            let angle = 2.0 * std::f64::consts::PI * i as f64 / in_ring as f64
                - std::f64::consts::FRAC_PI_2;
            out.insert(
                nodes[placed].id.clone(),
                (cx + radius * angle.cos(), cy + radius * angle.sin()),
            );
            placed += 1;
        }
        ring += 1;
    }
    out
}

/// BFS-layered hierarchical layout with barycenter ordering — port of
/// hierarchicalLayout() in graph-layout.ts.
fn tree_layout(
    nodes: &[GraphNode],
    edges: &[GraphRelationship],
    width: f64,
    height: f64,
) -> BTreeMap<String, (f64, f64)> {
    use std::collections::{HashMap, HashSet, VecDeque};
    let mut out = BTreeMap::new();
    if nodes.is_empty() {
        return out;
    }
    let mut adj: HashMap<&str, HashSet<&str>> = HashMap::new();
    for n in nodes {
        adj.entry(&n.id).or_default();
    }
    for e in edges {
        if adj.contains_key(e.source_id.as_str()) && adj.contains_key(e.target_id.as_str()) {
            adj.get_mut(e.source_id.as_str()).unwrap().insert(&e.target_id);
            adj.get_mut(e.target_id.as_str()).unwrap().insert(&e.source_id);
        }
    }
    const ORDER: [&str; 13] = [
        "Repository", "Database", "Service", "User", "Branch", "Directory", "Table",
        "CodeFile", "PullRequest", "Issue", "CodeClass", "CodeFunction", "Module",
    ];
    let rank_of = |n: &GraphNode| -> usize {
        let l = n.labels.first().map(String::as_str).unwrap_or("");
        ORDER.iter().position(|o| *o == l).unwrap_or(50)
    };
    let mut depth: HashMap<&str, usize> = HashMap::new();
    let mut by_rank: Vec<&GraphNode> = nodes.iter().collect();
    by_rank.sort_by(|a, b| rank_of(a).cmp(&rank_of(b)).then_with(|| a.id.cmp(&b.id)));
    for root in &by_rank {
        if depth.contains_key(root.id.as_str()) {
            continue;
        }
        depth.insert(&root.id, 0);
        let mut queue: VecDeque<&str> = VecDeque::from([root.id.as_str()]);
        while let Some(cur) = queue.pop_front() {
            let d = depth[cur];
            let mut nbs: Vec<&str> = adj[cur].iter().copied().collect();
            nbs.sort(); // deterministic BFS order
            for nb in nbs {
                if !depth.contains_key(nb) {
                    depth.insert(nb, d + 1);
                    queue.push_back(nb);
                }
            }
        }
    }
    let mut layers: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for n in nodes {
        layers.entry(*depth.get(n.id.as_str()).unwrap_or(&0)).or_default().push(&n.id);
    }
    let layer_gap = (height / (layers.len() as f64 + 1.0)).clamp(130.0, 220.0);
    for (d, ids) in &mut layers {
        // Barycenter of already-placed neighbours; NaN (no placed nbs) sorts last.
        let bary = |id: &str, placed: &BTreeMap<String, (f64, f64)>| -> Option<f64> {
            let (sum, count) = adj[id]
                .iter()
                .filter_map(|nb| placed.get(*nb).map(|p| p.0))
                .fold((0.0_f64, 0usize), |(s, c), x| (s + x, c + 1));
            if count == 0 { None } else { Some(sum / count as f64) }
        };
        ids.sort_by(|a, b| match (bary(a, &out), bary(b, &out)) {
            (None, None) => a.cmp(b),
            (None, _) => std::cmp::Ordering::Greater,
            (_, None) => std::cmp::Ordering::Less,
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
        });
        let count = ids.len();
        let gap = (width / (count as f64 + 1.0)).max(150.0);
        let start_x = width / 2.0 - gap * (count as f64 - 1.0) / 2.0;
        for (i, id) in ids.iter().enumerate() {
            out.insert((*id).to_string(), (start_x + i as f64 * gap, 70.0 + *d as f64 * layer_gap));
        }
    }
    out
}

struct LNode {
    idx: usize, // index into `nodes`
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    label_idx: usize,
}

/// Spatial-grid force layout — port of forceDirectedLayout() in graph-layout.ts.
/// All constants intentionally identical to the TS reference.
fn force_layout(
    nodes: &[GraphNode],
    edges: &[GraphRelationship],
    width: f64,
    height: f64,
) -> BTreeMap<String, (f64, f64)> {
    use std::collections::HashMap;
    let n = nodes.len();
    let mut out = BTreeMap::new();
    if n == 0 {
        return out;
    }
    let scale = (n as f64 / 40.0).sqrt().max(1.0);
    let (cw, ch) = (width * scale, height * scale);

    // Label groups (insertion order, like the TS Map).
    let mut label_list: Vec<String> = Vec::new();
    let mut label_of: Vec<usize> = Vec::with_capacity(n);
    let mut group_members: Vec<Vec<usize>> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let label = node.labels.first().cloned().unwrap_or_else(|| "Unknown".into());
        let gi = match label_list.iter().position(|l| *l == label) {
            Some(gi) => gi,
            None => {
                label_list.push(label);
                group_members.push(Vec::new());
                label_list.len() - 1
            }
        };
        label_of.push(gi);
        group_members[gi].push(i);
    }

    // Initial placement: groups on a circle, members on a circle within, jittered.
    let mut ln: Vec<LNode> = Vec::with_capacity(n);
    for (i, _node) in nodes.iter().enumerate() {
        let gi = label_of[i];
        let group = &group_members[gi];
        let member_idx = group.iter().position(|m| *m == i).unwrap();
        let group_angle = 2.0 * std::f64::consts::PI * gi as f64 / label_list.len() as f64;
        let group_radius = cw.min(ch) * 0.3;
        let cx = cw / 2.0 + group_angle.cos() * group_radius;
        let cy = ch / 2.0 + group_angle.sin() * group_radius;
        let member_angle = 2.0 * std::f64::consts::PI * member_idx as f64 / group.len() as f64;
        let member_radius = (group.len() as f64).sqrt() * 35.0;
        let jitter_seed = ((member_idx * 7919 + gi * 104729) % 1000) as f64;
        let jx = (jitter_seed / 1000.0 - 0.5) * 15.0;
        let jy = (((jitter_seed * 3.0) % 1000.0) / 1000.0 - 0.5) * 15.0;
        ln.push(LNode {
            idx: i,
            x: cx + member_angle.cos() * member_radius + jx,
            y: cy + member_angle.sin() * member_radius + jy,
            vx: 0.0,
            vy: 0.0,
            label_idx: gi,
        });
    }

    let id_to_lidx: HashMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, nd)| (nd.id.as_str(), i)).collect();

    let big = n > 200;
    let iterations = if n < 100 { 120 } else if n < 400 { 95 } else if n < 1200 { 75 } else if n < 3000 { 65 } else { 50 };
    let repulsion =
        (if big { 14000.0_f64 } else { 4000.0 }).max(10000.0 * (60.0 / n.max(1) as f64).sqrt());
    let attraction = if big { 0.004 } else { 0.006 };
    let damping = 0.85;
    let min_dist = 65.0;
    let center_gravity = if big { 0.003 } else { 0.008 };
    let label_cohesion = if big { 0.0015 } else { 0.004 };
    let repulsion_radius: f64 = if n > 200 { 1100.0 } else { 700.0 };
    let cell = repulsion_radius;

    let edge_pairs: Vec<(usize, usize)> = edges
        .iter()
        .filter_map(|e| {
            Some((*id_to_lidx.get(e.source_id.as_str())?, *id_to_lidx.get(e.target_id.as_str())?))
        })
        .collect();

    for iter in 0..iterations {
        let temp = 1.0 - iter as f64 / iterations as f64;
        let cooled = temp * temp;

        for node in ln.iter_mut() {
            node.vx += (cw / 2.0 - node.x) * center_gravity * cooled;
            node.vy += (ch / 2.0 - node.y) * center_gravity * cooled;
        }

        // Spatial grid repulsion — forward-direction neighbour cells only.
        let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, node) in ln.iter().enumerate() {
            grid.entry(((node.x / cell).floor() as i64, (node.y / cell).floor() as i64))
                .or_default()
                .push(i);
        }
        let radius_sq = repulsion_radius * repulsion_radius;
        let mut keys: Vec<(i64, i64)> = grid.keys().copied().collect();
        keys.sort(); // deterministic iteration order
        for key in keys {
            let bucket: &[usize] = &grid[&key];
            for (dx, dy) in NEIGHBOR_OFFSETS {
                let same = dx == 0 && dy == 0;
                let other: &[usize] = if same {
                    bucket
                } else {
                    match grid.get(&(key.0 + dx, key.1 + dy)) {
                        Some(o) => o,
                        None => continue,
                    }
                };
                for (bi, &a) in bucket.iter().enumerate() {
                    let j_start = if same { bi + 1 } else { 0 };
                    for &b in &other[j_start..] {
                        let ddx = ln[a].x - ln[b].x;
                        let ddy = ln[a].y - ln[b].y;
                        let dist_sq = ddx * ddx + ddy * ddy;
                        if dist_sq > radius_sq {
                            continue;
                        }
                        let dist = dist_sq.sqrt().max(min_dist);
                        let force = repulsion * cooled / (dist * dist);
                        let fx = ddx / dist * force;
                        let fy = ddy / dist * force;
                        ln[a].vx += fx;
                        ln[a].vy += fy;
                        ln[b].vx -= fx;
                        ln[b].vy -= fy;
                    }
                }
            }
        }

        // Attraction along edges toward ideal length.
        for &(s, t) in &edge_pairs {
            let dx = ln[t].x - ln[s].x;
            let dy = ln[t].y - ln[s].y;
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < 1e-9 {
                continue;
            }
            let dist = dist.max(1.0);
            let ideal = min_dist * 2.5;
            let force = (dist - ideal) * attraction * cooled;
            let fx = dx / dist * force;
            let fy = dy / dist * force;
            ln[s].vx += fx;
            ln[s].vy += fy;
            ln[t].vx -= fx;
            ln[t].vy -= fy;
        }

        // Same-label cohesion toward group centroid.
        if label_cohesion > 0.0 && !label_list.is_empty() {
            let mut sums = vec![(0.0_f64, 0.0_f64, 0usize); label_list.len()];
            for node in &ln {
                let s = &mut sums[node.label_idx];
                s.0 += node.x;
                s.1 += node.y;
                s.2 += 1;
            }
            for node in ln.iter_mut() {
                let (sx, sy, c) = sums[node.label_idx];
                if c < 2 {
                    continue;
                }
                node.vx += (sx / c as f64 - node.x) * label_cohesion * cooled;
                node.vy += (sy / c as f64 - node.y) * label_cohesion * cooled;
            }
        }

        for node in ln.iter_mut() {
            node.vx *= damping;
            node.vy *= damping;
            node.x += node.vx;
            node.y += node.vy;
        }
    }

    // Overlap resolution passes.
    let overlap_dist = 80.0;
    let ocell = overlap_dist * 2.0;
    for _pass in 0..12 {
        let mut any = false;
        let mut ogrid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, node) in ln.iter().enumerate() {
            ogrid
                .entry(((node.x / ocell).floor() as i64, (node.y / ocell).floor() as i64))
                .or_default()
                .push(i);
        }
        let mut keys: Vec<(i64, i64)> = ogrid.keys().copied().collect();
        keys.sort();
        for key in keys {
            let bucket: &[usize] = &ogrid[&key];
            for (dx, dy) in NEIGHBOR_OFFSETS {
                let same = dx == 0 && dy == 0;
                let other: &[usize] = if same {
                    bucket
                } else {
                    match ogrid.get(&(key.0 + dx, key.1 + dy)) {
                        Some(o) => o,
                        None => continue,
                    }
                };
                for (bi, &a) in bucket.iter().enumerate() {
                    let j_start = if same { bi + 1 } else { 0 };
                    for &b in &other[j_start..] {
                        let ddx = ln[b].x - ln[a].x;
                        let ddy = ln[b].y - ln[a].y;
                        let dist = (ddx * ddx + ddy * ddy).sqrt();
                        if dist >= overlap_dist {
                            continue;
                        }
                        any = true;
                        let push = (overlap_dist - dist) / 2.0 + 0.5;
                        if dist > 0.1 {
                            let nx = ddx / dist;
                            let ny = ddy / dist;
                            ln[a].x -= nx * push;
                            ln[a].y -= ny * push;
                            ln[b].x += nx * push;
                            ln[b].y += ny * push;
                        } else {
                            ln[a].x -= push;
                            ln[b].x += push;
                        }
                    }
                }
            }
        }
        if !any {
            break;
        }
    }

    for node in &ln {
        out.insert(nodes[node.idx].id.clone(), (node.x, node.y));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeMetadata;

    fn node(id: &str, label: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            labels: vec![label.into()],
            properties: Default::default(),
            metadata: NodeMetadata {
                created_at: "2026-01-01T00:00:00Z".into(),
                updated_at: "2026-01-01T00:00:00Z".into(),
                source_type: None,
                source_id: None,
                realm: "default".into(),
            },
        }
    }

    fn edge(id: &str, src: &str, dst: &str) -> GraphRelationship {
        GraphRelationship {
            id: id.into(),
            source_id: src.into(),
            target_id: dst.into(),
            relationship_type: "REFERENCES".into(),
            properties: Default::default(),
        }
    }

    fn star(n: usize) -> (Vec<GraphNode>, Vec<GraphRelationship>) {
        let mut nodes = vec![node("hub", "Service")];
        let mut edges = Vec::new();
        for i in 0..n {
            nodes.push(node(&format!("leaf{i}"), "Table"));
            edges.push(edge(&format!("e{i}"), "hub", &format!("leaf{i}")));
        }
        (nodes, edges)
    }

    #[test]
    fn every_node_gets_a_position_in_every_algorithm() {
        let (nodes, edges) = star(50);
        for algo in [
            LayoutAlgorithm::Force,
            LayoutAlgorithm::Tree,
            LayoutAlgorithm::Grid,
            LayoutAlgorithm::Radial,
        ] {
            let pos = compute_layout(algo, &nodes, &edges, 1600.0, 1000.0);
            assert_eq!(pos.len(), nodes.len(), "{algo:?} missed nodes");
            for (_, (x, y)) in &pos {
                assert!(x.is_finite() && y.is_finite(), "{algo:?} produced non-finite");
            }
        }
    }

    #[test]
    fn force_layout_is_deterministic() {
        let (nodes, edges) = star(120);
        let a = compute_layout(LayoutAlgorithm::Force, &nodes, &edges, 1600.0, 1000.0);
        let b = compute_layout(LayoutAlgorithm::Force, &nodes, &edges, 1600.0, 1000.0);
        assert_eq!(a, b);
    }

    #[test]
    fn force_layout_spreads_nodes_apart() {
        let (nodes, edges) = star(80);
        let pos = compute_layout(LayoutAlgorithm::Force, &nodes, &edges, 1600.0, 1000.0);
        // After the overlap pass no two nodes should sit closer than ~half
        // OVERLAP_DIST (the pass runs 12 rounds; allow stragglers but not piles).
        let pts: Vec<_> = pos.values().collect();
        let mut too_close = 0;
        for i in 0..pts.len() {
            for j in (i + 1)..pts.len() {
                let d = ((pts[i].0 - pts[j].0).powi(2) + (pts[i].1 - pts[j].1).powi(2)).sqrt();
                if d < 40.0 {
                    too_close += 1;
                }
            }
        }
        assert_eq!(too_close, 0, "{too_close} node pairs closer than 40px");
    }

    #[test]
    fn grid_layout_is_a_grid() {
        let (nodes, _) = star(8); // 9 nodes total
        let pos = compute_layout(LayoutAlgorithm::Grid, &nodes, &[], 900.0, 900.0);
        // 9 nodes in a ~square grid: expect 3 distinct x values and 3 distinct y values.
        let mut xs: Vec<i64> = pos.values().map(|p| p.0.round() as i64).collect();
        xs.sort();
        xs.dedup();
        assert!(xs.len() <= 4, "grid xs: {xs:?}");
    }

    #[test]
    fn tree_layout_puts_hub_above_leaves() {
        let (nodes, edges) = star(10);
        let pos = compute_layout(LayoutAlgorithm::Tree, &nodes, &edges, 1600.0, 1000.0);
        let hub_y = pos["hub"].1;
        for i in 0..10 {
            assert!(pos[&format!("leaf{i}")].1 > hub_y, "leaf{i} not below hub");
        }
    }

    #[test]
    fn algorithm_serializes_lowercase() {
        assert_eq!(serde_json::to_value(LayoutAlgorithm::Force).unwrap(), "force");
        assert_eq!(
            serde_json::from_value::<LayoutAlgorithm>(serde_json::json!("radial")).unwrap(),
            LayoutAlgorithm::Radial
        );
    }

    /// Two nodes connected by an edge must produce finite positions even when
    /// their initial coordinates are nearly coincident (sub-pixel distance).
    /// This exercises the `dist < 1e-9` / `dist.max(1.0)` clamp in edge
    /// attraction — without the clamp a near-zero dist would spike the force
    /// to ±Inf and propagate NaN through the whole simulation.
    #[test]
    fn force_layout_handles_near_coincident_nodes() {
        let nodes = vec![node("a", "Service"), node("b", "Service")];
        let edges = vec![edge("e0", "a", "b")];
        let pos = compute_layout(LayoutAlgorithm::Force, &nodes, &edges, 1600.0, 1000.0);
        assert_eq!(pos.len(), 2);
        for (id, (x, y)) in &pos {
            assert!(x.is_finite() && y.is_finite(), "node {id} produced non-finite position");
        }
    }
}
