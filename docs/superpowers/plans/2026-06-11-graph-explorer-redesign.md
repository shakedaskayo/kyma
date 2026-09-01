# Graph Explorer Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render the full graph (tens of thousands of nodes) every time at 60fps with directional edges, relationship references, and full-bleed navigation chrome (⌘K, inspector, legend dock, minimap, breadcrumbs, keyboard walking).

**Architecture:** Server-side deterministic layout (Rust port of the TS algorithms) cached per graph version, served through a new paginated `/v1/graph/:graph/export` endpoint that includes node positions. The client builds one graphology instance per view and renders it with Sigma.js v3 (WebGL) using quiet-arrows/loud-focus edge styling and zoom-based LOD. The old canvas renderer stays as a no-WebGL fallback.

**Tech Stack:** Rust (axum, pensieve-graph crate), TypeScript, React, sigma@^3, graphology, @sigma/edge-curve, @sigma/node-image, zustand, React Query, vitest, Tailwind (`pv-` prefix).

**Spec:** `docs/superpowers/specs/2026-06-11-graph-explorer-redesign-design.md`

**Run commands from repo root.** Rust tests: `cargo test -p <crate>`. JS: `pnpm --filter @pensieve-ai/client test`, `pnpm --filter @pensieve-ai/react test`, `pnpm --filter @pensieve-ai/react typecheck`.

---

## Phase A — Server: layout + export

### Task 1: Rust layout module in pensieve-graph

Port the four deterministic layout algorithms from `packages/client/src/graph-layout.ts` (the reference implementation — keep semantics identical) into the `pensieve-graph` crate.

**Files:**
- Create: `crates/pensieve-graph/src/layout.rs`
- Modify: `crates/pensieve-graph/src/lib.rs` (add `pub mod layout;` + re-export `layout::{LayoutAlgorithm, compute_layout}`)
- Test: inline `#[cfg(test)]` in `layout.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/pensieve-graph/src/layout.rs` with the types, an unimplemented body (`todo!()`), and these tests at the bottom:

```rust
//! Deterministic layout algorithms, ported from packages/client/src/graph-layout.ts.
//! Positions are computed server-side once per graph version and cached.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{GraphNode, GraphRelationship};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayoutAlgorithm {
    Force,
    Tree,
    Grid,
    Radial,
}

impl Default for LayoutAlgorithm {
    fn default() -> Self {
        Self::Force
    }
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
    todo!()
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
}
```

Add to `crates/pensieve-graph/src/lib.rs` next to the other `pub mod` lines:

```rust
pub mod layout;
pub use layout::{compute_layout, LayoutAlgorithm, LAYOUT_HEIGHT, LAYOUT_WIDTH};
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pensieve-graph layout`
Expected: panics at `todo!()` (compiles, tests fail).

- [ ] **Step 3: Implement the algorithms**

Replace the `todo!()` body and add the four implementations. This is a faithful port of `packages/client/src/graph-layout.ts` — read that file side-by-side while porting; constants and force formulas must match:

```rust
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
    by_rank.sort_by_key(|n| (rank_of(n), n.id.clone()));
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
            let xs: Vec<f64> =
                adj[id].iter().filter_map(|nb| placed.get(*nb).map(|p| p.0)).collect();
            if xs.is_empty() { None } else { Some(xs.iter().sum::<f64>() / xs.len() as f64) }
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
        let offsets: [(i64, i64); 5] = [(0, 0), (1, 0), (0, 1), (1, 1), (-1, 1)];
        let mut keys: Vec<(i64, i64)> = grid.keys().copied().collect();
        keys.sort(); // deterministic iteration order
        for key in keys {
            let bucket = grid[&key].clone();
            for (dx, dy) in offsets {
                let same = dx == 0 && dy == 0;
                let other: &[usize] = if same {
                    &bucket
                } else {
                    match grid.get(&(key.0 + dx, key.1 + dy)) {
                        Some(o) => o,
                        None => continue,
                    }
                };
                let other = other.to_vec();
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
            if dist == 0.0 {
                continue;
            }
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
        let offsets: [(i64, i64); 5] = [(0, 0), (1, 0), (0, 1), (1, 1), (-1, 1)];
        let mut keys: Vec<(i64, i64)> = ogrid.keys().copied().collect();
        keys.sort();
        for key in keys {
            let bucket = ogrid[&key].clone();
            for (dx, dy) in offsets {
                let same = dx == 0 && dy == 0;
                let other = if same {
                    bucket.clone()
                } else {
                    match ogrid.get(&(key.0 + dx, key.1 + dy)) {
                        Some(o) => o.clone(),
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
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pensieve-graph layout`
Expected: all 6 tests PASS. Also run `cargo clippy -p pensieve-graph` — no new warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-graph/src/layout.rs crates/pensieve-graph/src/lib.rs
git commit -m "feat(graph): server-side deterministic layout algorithms in pensieve-graph"
```

---

### Task 2: Export wire types + layout cache

**Files:**
- Modify: `crates/pensieve-graph/src/types.rs` (add `PositionedNode`, `GraphExportPage`)
- Create: `crates/pensieve-server/src/graph_layout_cache.rs`
- Modify: `crates/pensieve-server/src/lib.rs` (declare `pub mod graph_layout_cache;` next to the other module declarations; find the exact spot with `rg -n "mod graph_handler" crates/pensieve-server/src/lib.rs`)
- Test: inline `#[cfg(test)]` in `graph_layout_cache.rs`

- [ ] **Step 1: Add wire types to pensieve-graph**

Append to `crates/pensieve-graph/src/types.rs`:

```rust
/// Node + its precomputed layout position, as served by `/v1/graph/:graph/export`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionedNode {
    #[serde(flatten)]
    pub node: GraphNode,
    pub x: f64,
    pub y: f64,
}

/// One page of the full-graph export. Node pages stream first, then edge
/// pages; `next_cursor` is `None` on the final page. `layout_status` is
/// `"computing"` (with empty nodes/edges) while a large layout is being
/// computed in the background — clients poll until `"ready"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphExportPage {
    pub layout_status: String,
    pub layout_id: String,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub nodes: Vec<PositionedNode>,
    pub edges: Vec<GraphRelationship>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
```

- [ ] **Step 2: Write the failing cache tests**

Create `crates/pensieve-server/src/graph_layout_cache.rs`:

```rust
//! Cache of laid-out full graphs keyed by (database, graph, realm, algorithm).
//! Entries are fingerprinted by (total_nodes, total_relationships) — a cheap
//! version proxy. Pagination slices a Ready entry; cursors embed the
//! layout_id so pages stay consistent even if the graph mutates mid-paging.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pensieve_graph::{GraphExportPage, GraphRelationship, PositionedNode};

pub const PAGE_SIZE_DEFAULT: usize = 10_000;
/// Above this node count, layout is computed in a background task and the
/// endpoint reports `layout_status: "computing"` until it lands.
pub const SYNC_COMPUTE_MAX_NODES: usize = 20_000;
const MAX_ENTRIES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub database: String,
    pub graph: String,
    pub realm: Option<String>,
    pub algorithm: pensieve_graph::LayoutAlgorithm,
}

#[derive(Debug)]
pub struct LaidOutGraph {
    pub layout_id: String,
    pub fingerprint: (usize, usize),
    pub nodes: Vec<PositionedNode>,
    pub edges: Vec<GraphRelationship>,
}

#[derive(Debug, Clone)]
pub enum CacheState {
    Computing,
    Ready(Arc<LaidOutGraph>),
}

#[derive(Default)]
pub struct LayoutCache {
    /// Primary store + LRU order (front = oldest).
    inner: Mutex<(HashMap<CacheKey, CacheState>, Vec<CacheKey>, HashMap<String, Arc<LaidOutGraph>>)>,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Deterministic id for a (key, fingerprint) pair.
    pub fn layout_id(key: &CacheKey, fingerprint: (usize, usize)) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        key.hash(&mut h);
        fingerprint.hash(&mut h);
        format!("L{:016x}", h.finish())
    }

    /// Current state for `key` IF its fingerprint still matches; stale Ready
    /// entries are evicted and `None` is returned (caller recomputes).
    pub fn get_fresh(&self, key: &CacheKey, fingerprint: (usize, usize)) -> Option<CacheState> {
        let mut g = self.inner.lock().unwrap();
        let (map, lru, by_id) = &mut *g;
        match map.get(key) {
            Some(CacheState::Computing) => Some(CacheState::Computing),
            Some(CacheState::Ready(l)) if l.fingerprint == fingerprint => {
                // touch LRU
                lru.retain(|k| k != key);
                lru.push(key.clone());
                Some(CacheState::Ready(l.clone()))
            }
            Some(CacheState::Ready(l)) => {
                by_id.remove(&l.layout_id);
                map.remove(key);
                lru.retain(|k| k != key);
                None
            }
            None => None,
        }
    }

    /// Mark `key` as computing (idempotent). Returns false if it was already
    /// computing (so only one task computes).
    pub fn begin_compute(&self, key: &CacheKey) -> bool {
        let mut g = self.inner.lock().unwrap();
        let (map, _, _) = &mut *g;
        if matches!(map.get(key), Some(CacheState::Computing)) {
            return false;
        }
        map.insert(key.clone(), CacheState::Computing);
        true
    }

    pub fn insert_ready(&self, key: CacheKey, laid: LaidOutGraph) -> Arc<LaidOutGraph> {
        let laid = Arc::new(laid);
        let mut g = self.inner.lock().unwrap();
        let (map, lru, by_id) = &mut *g;
        by_id.insert(laid.layout_id.clone(), laid.clone());
        map.insert(key.clone(), CacheState::Ready(laid.clone()));
        lru.retain(|k| k != &key);
        lru.push(key);
        while lru.len() > MAX_ENTRIES {
            let evict = lru.remove(0);
            if let Some(CacheState::Ready(l)) = map.remove(&evict) {
                by_id.remove(&l.layout_id);
            }
        }
        laid
    }

    /// Drop a Computing marker after a failed compute so the next request retries.
    pub fn abort_compute(&self, key: &CacheKey) {
        let mut g = self.inner.lock().unwrap();
        if matches!(g.0.get(key), Some(CacheState::Computing)) {
            g.0.remove(key);
        }
    }

    /// Resolve a paging cursor's layout_id to its graph (for pages 2+).
    pub fn by_layout_id(&self, layout_id: &str) -> Option<Arc<LaidOutGraph>> {
        self.inner.lock().unwrap().2.get(layout_id).cloned()
    }
}

/// Cursor grammar: `"{layout_id}:n:{offset}"` (node pages) then
/// `"{layout_id}:e:{offset}"` (edge pages). Returns None on garbage.
pub fn parse_cursor(cursor: &str) -> Option<(String, char, usize)> {
    let mut parts = cursor.rsplitn(3, ':');
    let offset: usize = parts.next()?.parse().ok()?;
    let kind = parts.next()?.chars().next().filter(|c| *c == 'n' || *c == 'e')?;
    let id = parts.next()?.to_string();
    if id.is_empty() {
        return None;
    }
    Some((id, kind, offset))
}

/// Slice one page out of a laid-out graph. Node pages first, then edge pages.
pub fn slice_page(g: &LaidOutGraph, kind: char, offset: usize, page_size: usize) -> GraphExportPage {
    let mut page = GraphExportPage {
        layout_status: "ready".into(),
        layout_id: g.layout_id.clone(),
        total_nodes: g.nodes.len(),
        total_edges: g.edges.len(),
        nodes: Vec::new(),
        edges: Vec::new(),
        next_cursor: None,
    };
    match kind {
        'n' => {
            let end = (offset + page_size).min(g.nodes.len());
            page.nodes = g.nodes[offset.min(g.nodes.len())..end].to_vec();
            page.next_cursor = if end < g.nodes.len() {
                Some(format!("{}:n:{}", g.layout_id, end))
            } else if !g.edges.is_empty() {
                Some(format!("{}:e:0", g.layout_id))
            } else {
                None
            };
        }
        _ => {
            let end = (offset + page_size).min(g.edges.len());
            page.edges = g.edges[offset.min(g.edges.len())..end].to_vec();
            page.next_cursor =
                (end < g.edges.len()).then(|| format!("{}:e:{}", g.layout_id, end));
        }
    }
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use pensieve_graph::{GraphNode, NodeMetadata};

    fn laid(n: usize, e: usize) -> LaidOutGraph {
        let nodes = (0..n)
            .map(|i| PositionedNode {
                node: GraphNode {
                    id: format!("n{i}"),
                    labels: vec!["Table".into()],
                    properties: Default::default(),
                    metadata: NodeMetadata {
                        created_at: String::new(),
                        updated_at: String::new(),
                        source_type: None,
                        source_id: None,
                        realm: "default".into(),
                    },
                },
                x: i as f64,
                y: 0.0,
            })
            .collect();
        let edges = (0..e)
            .map(|i| GraphRelationship {
                id: format!("e{i}"),
                source_id: format!("n{}", i % n.max(1)),
                target_id: format!("n{}", (i + 1) % n.max(1)),
                relationship_type: "REFERENCES".into(),
                properties: Default::default(),
            })
            .collect();
        LaidOutGraph { layout_id: "Labc".into(), fingerprint: (n, e), nodes, edges }
    }

    #[test]
    fn pages_walk_nodes_then_edges_to_completion() {
        let g = laid(25, 7);
        let p1 = slice_page(&g, 'n', 0, 10);
        assert_eq!(p1.nodes.len(), 10);
        assert_eq!(p1.next_cursor.as_deref(), Some("Labc:n:10"));
        let p3 = slice_page(&g, 'n', 20, 10);
        assert_eq!(p3.nodes.len(), 5);
        assert_eq!(p3.next_cursor.as_deref(), Some("Labc:e:0"));
        let p4 = slice_page(&g, 'e', 0, 10);
        assert_eq!(p4.edges.len(), 7);
        assert_eq!(p4.next_cursor, None);
        assert_eq!(p4.total_nodes, 25);
        assert_eq!(p4.total_edges, 7);
    }

    #[test]
    fn cursor_roundtrip_and_garbage() {
        assert_eq!(parse_cursor("Labc:n:10"), Some(("Labc".into(), 'n', 10)));
        assert_eq!(parse_cursor("Labc:e:0"), Some(("Labc".into(), 'e', 0)));
        assert_eq!(parse_cursor("nonsense"), None);
        assert_eq!(parse_cursor("x:q:5"), None);
        assert_eq!(parse_cursor(":n:5"), None);
    }

    #[test]
    fn stale_fingerprint_evicts() {
        let cache = LayoutCache::new();
        let key = CacheKey {
            database: "db".into(),
            graph: "g".into(),
            realm: None,
            algorithm: pensieve_graph::LayoutAlgorithm::Force,
        };
        cache.insert_ready(key.clone(), laid(5, 2));
        assert!(matches!(cache.get_fresh(&key, (5, 2)), Some(CacheState::Ready(_))));
        assert!(cache.get_fresh(&key, (6, 2)).is_none(), "stale entry must evict");
        assert!(cache.by_layout_id("Labc").is_none(), "by_id index must evict too");
    }

    #[test]
    fn begin_compute_is_exclusive() {
        let cache = LayoutCache::new();
        let key = CacheKey {
            database: "db".into(),
            graph: "g".into(),
            realm: None,
            algorithm: pensieve_graph::LayoutAlgorithm::Force,
        };
        assert!(cache.begin_compute(&key));
        assert!(!cache.begin_compute(&key));
        cache.abort_compute(&key);
        assert!(cache.begin_compute(&key));
    }
}
```

Note: `CacheKey` derives `Hash` — `pensieve_graph::LayoutAlgorithm` already derives `Hash` (Task 1).

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p pensieve-server graph_layout_cache`
Expected: compile error (module not declared) — add `pub mod graph_layout_cache;` to `crates/pensieve-server/src/lib.rs`, re-run, tests should now PASS (this module is written test-with-implementation in one file; the failing state is the missing module declaration).

- [ ] **Step 4: Run full crate checks**

Run: `cargo test -p pensieve-server graph_layout_cache && cargo clippy -p pensieve-server`
Expected: 4 tests PASS, no new clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-graph/src/types.rs crates/pensieve-server/src/graph_layout_cache.rs crates/pensieve-server/src/lib.rs
git commit -m "feat(graph): export wire types + layout cache with fingerprint invalidation"
```

---

### Task 3: `/v1/graph/:graph/export` endpoint

**Files:**
- Modify: `crates/pensieve-server/src/graph_handler.rs` (new query struct, handler, route)
- Modify: `crates/pensieve-server/src/lib.rs` (add `layout_cache: Arc<graph_layout_cache::LayoutCache>` to `QueryState`; find the struct with `rg -n "struct QueryState" crates/pensieve-server/src`; initialize with `Arc::new(LayoutCache::new())` at every construction site — find them with `rg -n "QueryState\s*\{" crates/pensieve-server crates/pensieve-cli`)
- Test: `crates/pensieve-server/tests/` — follow the existing integration-test pattern for graph endpoints (find with `rg -ln "overview" crates/pensieve-server/tests/`); if no graph integration harness exists, the unit tests from Task 2 plus a manual `curl` verification in Step 4 are the test surface.

- [ ] **Step 1: Add the handler**

In `crates/pensieve-server/src/graph_handler.rs`, add below `NeighborsBody`:

```rust
#[derive(Deserialize)]
struct ExportQuery {
    realm: Option<String>,
    #[serde(default)]
    algorithm: pensieve_graph::LayoutAlgorithm,
    cursor: Option<String>,
    #[serde(default = "default_export_page_size")]
    page_size: usize,
}
fn default_export_page_size() -> usize {
    crate::graph_layout_cache::PAGE_SIZE_DEFAULT
}
```

And the handler (place after `neighbors`):

```rust
/// Full-graph export with precomputed layout positions, paginated.
/// First call (no cursor): checks cache freshness via stats fingerprint,
/// computes layout (inline for small graphs, background task for large),
/// and returns the first node page or `layout_status: "computing"`.
/// Subsequent calls pass the returned cursor and are served from the cache.
async fn export(
    State(state): State<QueryState>,
    principal: Option<Extension<crate::auth::Principal>>,
    Path(graph): Path<String>,
    Query(q): Query<ExportQuery>,
    headers: axum::http::HeaderMap,
) -> Response {
    use crate::graph_layout_cache::{
        parse_cursor, slice_page, CacheKey, CacheState, LaidOutGraph, LayoutCache,
        SYNC_COMPUTE_MAX_NODES,
    };
    let db = db_from_headers(&headers);
    if let Err(r) = enforce_scope(principal.as_deref(), &db) {
        return r;
    }
    let page_size = q.page_size.clamp(100, 50_000);

    // Pages 2+: serve straight from the layout_id in the cursor.
    if let Some(cursor) = &q.cursor {
        let Some((layout_id, kind, offset)) = parse_cursor(cursor) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": {"code": "bad_cursor", "message": "malformed cursor"}})),
            )
                .into_response();
        };
        return match state.layout_cache.by_layout_id(&layout_id) {
            Some(laid) => (StatusCode::OK, Json(slice_page(&laid, kind, offset, page_size)))
                .into_response(),
            // Evicted mid-paging — client restarts from cursor=None.
            None => (
                StatusCode::GONE,
                Json(serde_json::json!({"error": {"code": "layout_evicted", "message": "layout evicted; restart export"}})),
            )
                .into_response(),
        };
    }

    // First page: resolve provider + fingerprint.
    let allowed = principal.as_deref().and_then(|p| p.allowed_databases.clone());
    let p = match resolve(&state, &graph, &db, allowed).await {
        Ok(p) => p,
        Err(r) => return r,
    };
    let stats = match p.stats(q.realm.as_deref()).await {
        Ok(s) => s,
        Err(e) => return err500(e),
    };
    let fingerprint = (stats.total_nodes, stats.total_relationships);
    let key = CacheKey {
        database: db.clone(),
        graph: graph.clone(),
        realm: q.realm.clone(),
        algorithm: q.algorithm,
    };
    let computing = |key: &CacheKey, fp: (usize, usize)| {
        Json(pensieve_graph::GraphExportPage {
            layout_status: "computing".into(),
            layout_id: LayoutCache::layout_id(key, fp),
            total_nodes: fp.0,
            total_edges: fp.1,
            nodes: vec![],
            edges: vec![],
            next_cursor: None,
        })
    };

    match state.layout_cache.get_fresh(&key, fingerprint) {
        Some(CacheState::Ready(laid)) => {
            (StatusCode::OK, Json(slice_page(&laid, 'n', 0, page_size))).into_response()
        }
        Some(CacheState::Computing) => {
            (StatusCode::OK, computing(&key, fingerprint)).into_response()
        }
        None => {
            let compute = move |payload: pensieve_graph::GraphPayload,
                                key: &CacheKey|
                  -> LaidOutGraph {
                let positions = pensieve_graph::compute_layout(
                    key.algorithm,
                    &payload.nodes,
                    &payload.edges,
                    pensieve_graph::LAYOUT_WIDTH,
                    pensieve_graph::LAYOUT_HEIGHT,
                );
                let fp = (payload.nodes.len(), payload.edges.len());
                LaidOutGraph {
                    layout_id: LayoutCache::layout_id(key, fp),
                    fingerprint: fp,
                    nodes: payload
                        .nodes
                        .into_iter()
                        .map(|n| {
                            let (x, y) = positions.get(&n.id).copied().unwrap_or((0.0, 0.0));
                            pensieve_graph::PositionedNode { node: n, x, y }
                        })
                        .collect(),
                    edges: payload.edges,
                }
            };

            if stats.total_nodes <= SYNC_COMPUTE_MAX_NODES {
                // Small graph: compute inline and serve the first page now.
                let payload = match p.overview(q.realm.as_deref(), usize::MAX).await {
                    Ok(pl) => pl,
                    Err(e) => return err500(e),
                };
                let laid = state.layout_cache.insert_ready(key.clone(), compute(payload, &key));
                (StatusCode::OK, Json(slice_page(&laid, 'n', 0, page_size))).into_response()
            } else {
                // Large graph: background compute, report computing.
                if state.layout_cache.begin_compute(&key) {
                    let cache = state.layout_cache.clone();
                    let realm = q.realm.clone();
                    let bg_key = key.clone();
                    tokio::spawn(async move {
                        match p.overview(realm.as_deref(), usize::MAX).await {
                            Ok(payload) => {
                                let laid = compute(payload, &bg_key);
                                cache.insert_ready(bg_key, laid);
                            }
                            Err(e) => {
                                tracing::warn!("graph export layout failed: {e}");
                                cache.abort_compute(&bg_key);
                            }
                        }
                    });
                }
                (StatusCode::OK, computing(&key, fingerprint)).into_response()
            }
        }
    }
}
```

Add the route in `graph_router`:

```rust
        .route("/v1/graph/:graph/export", get(export))
```

Note: `ResolvedProvider` must be moved into the spawned task — it already owns its data (`SchemaGraphProvider`/`StoredGraphProvider` hold `Arc`s), so `p` moves into the closure fine. If the compiler complains about `Send`, wrap `p` in the spawn before any awaits on it.

- [ ] **Step 2: Compile + add integration coverage if the harness exists**

Run: `rg -ln "overview" crates/pensieve-server/tests/` — if a graph endpoint integration test exists, add an export case asserting: (a) first call returns `layout_status: "ready"` with positioned nodes for a small registered graph; (b) following `next_cursor` chains to `null` and the union of pages equals the overview payload; (c) a garbage cursor returns 400. Mirror the file's existing setup verbatim.

Run: `cargo build -p pensieve-server`
Expected: clean build.

- [ ] **Step 3: Run tests**

Run: `cargo test -p pensieve-server`
Expected: PASS (including any new integration cases).

- [ ] **Step 4: Manual smoke (if a dev deployment is running)**

```bash
curl -s "http://localhost:8080/v1/graph/schema/export?page_size=500" | head -c 400
```
Expected: JSON with `"layout_status":"ready"`, nodes carrying `x`/`y`.

- [ ] **Step 5: Commit**

```bash
git add crates/pensieve-server/src/graph_handler.rs crates/pensieve-server/src/lib.rs crates/pensieve-server/tests
git commit -m "feat(server): paginated full-graph export endpoint with cached server-side layout"
```

---

### Task 4: client `exportGraph` method

**Files:**
- Modify: `packages/client/src/graph.ts` (types + method — mirror `getOverview`'s structure exactly; read the file first)
- Modify: `packages/client/src/index.ts` (export new types if graph.ts types are re-exported there — check with `rg -n "GraphPayload" packages/client/src/index.ts`)
- Test: `packages/client/src/graph.test.ts` (append cases following the file's existing mock pattern)

- [ ] **Step 1: Write the failing test**

Append to `packages/client/src/graph.test.ts`, reusing the file's existing client/fetch-mock setup (read the top of the file and copy its helper):

```ts
describe("exportGraph", () => {
  it("requests /export with algorithm + cursor and parses positioned nodes", async () => {
    const page = {
      layout_status: "ready",
      layout_id: "Labc",
      total_nodes: 2,
      total_edges: 1,
      nodes: [
        { id: "a", labels: ["Table"], properties: {}, metadata: m(), x: 10, y: 20 },
        { id: "b", labels: ["Table"], properties: {}, metadata: m(), x: 30, y: 40 },
      ],
      edges: [],
      next_cursor: "Labc:e:0",
    };
    // use the same fetch-mock helper the other graph tests use:
    mockFetchOnce(page);
    const res = await client.graph.exportGraph({ graph: "g", algorithm: "force", pageSize: 500 });
    expect(lastFetchUrl()).toContain("/v1/graph/g/export");
    expect(lastFetchUrl()).toContain("algorithm=force");
    expect(lastFetchUrl()).toContain("page_size=500");
    expect(res.nodes[0].x).toBe(10);
    expect(res.next_cursor).toBe("Labc:e:0");
  });

  it("passes cursor through", async () => {
    mockFetchOnce({ layout_status: "ready", layout_id: "L", total_nodes: 0, total_edges: 0, nodes: [], edges: [], next_cursor: null });
    await client.graph.exportGraph({ graph: "g", cursor: "Labc:e:0" });
    expect(lastFetchUrl()).toContain("cursor=Labc%3Ae%3A0");
  });
});
```

(`m()`, `mockFetchOnce`, `lastFetchUrl` are stand-ins for the file's actual helpers — match whatever `getOverview`'s tests use. If helpers with these capabilities don't exist, write them in this test file.)

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/client test -- graph`
Expected: FAIL — `exportGraph is not a function`.

- [ ] **Step 3: Implement**

In `packages/client/src/graph.ts` add types next to `GraphPayload`:

```ts
export type PositionedGraphNode = GraphNode & { x: number; y: number };

export interface GraphExportPage {
  layout_status: "ready" | "computing";
  layout_id: string;
  total_nodes: number;
  total_edges: number;
  nodes: PositionedGraphNode[];
  edges: GraphRelationship[];
  next_cursor?: string | null;
}

export interface ExportGraphArgs {
  graph: string;
  realm?: string;
  algorithm?: LayoutAlgorithm;
  cursor?: string;
  pageSize?: number;
}
```

And the method on the graph API class, written in the same style as `getOverview` (same transport call, same query-param builder):

```ts
async exportGraph(args: ExportGraphArgs): Promise<GraphExportPage> {
  const params = new URLSearchParams();
  if (args.realm) params.set("realm", args.realm);
  if (args.algorithm) params.set("algorithm", args.algorithm);
  if (args.cursor) params.set("cursor", args.cursor);
  if (args.pageSize) params.set("page_size", String(args.pageSize));
  const qs = params.toString();
  return this.transport.request<GraphExportPage>(
    `/v1/graph/${encodeURIComponent(args.graph)}/export${qs ? `?${qs}` : ""}`,
  );
}
```

(Adapt `this.transport.request` to the exact request helper `getOverview` uses — keep it identical.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm --filter @pensieve-ai/client test && pnpm --filter @pensieve-ai/client typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/client/src/graph.ts packages/client/src/graph.test.ts packages/client/src/index.ts
git commit -m "feat(client): graph.exportGraph — paginated full-graph export with positions"
```

---

## Phase B — Client data layer

### Task 5: chunked export loading hook

**Files:**
- Create: `packages/react/src/graph/graph-export-merge.ts` (pure merge logic)
- Create: `packages/react/src/hooks/useGraphExport.ts`
- Modify: `packages/react/src/hooks/usePensieveGraph.ts` (extract + export `useGraphCoords` — the discovery section, lines 90–129, verbatim into a standalone hook used by both)
- Test: `packages/react/src/graph/graph-export-merge.test.ts`

- [ ] **Step 1: Write the failing merge tests**

```ts
import { describe, expect, it } from "vitest";
import { createExportAccumulator, mergeExportPage } from "./graph-export-merge";
import type { GraphExportPage } from "@pensieve-ai/client";

const meta = { created_at: "", updated_at: "", realm: "default" };
const pnode = (id: string, x: number, y: number) => ({
  id, labels: ["Table"], properties: {}, metadata: meta, x, y,
});
const edge = (id: string, s: string, t: string) => ({
  id, source_id: s, target_id: t, relationship_type: "REFERENCES", properties: {},
});
const page = (p: Partial<GraphExportPage>): GraphExportPage => ({
  layout_status: "ready", layout_id: "L", total_nodes: 0, total_edges: 0,
  nodes: [], edges: [], next_cursor: null, ...p,
});

describe("mergeExportPage", () => {
  it("namespaces nodes, records positions by composite id, dedups across pages", () => {
    const acc = createExportAccumulator();
    mergeExportPage(acc, page({ nodes: [pnode("a", 1, 2)] }), "db/g", "db");
    mergeExportPage(acc, page({ nodes: [pnode("a", 1, 2), pnode("b", 3, 4)] }), "db/g", "db");
    expect(acc.nodes).toHaveLength(2);
    expect(acc.nodes[0].namespace).toBe("db/g");
    expect(acc.nodes[0].database).toBe("db");
    expect(acc.positions.get("db/g::a")).toEqual({ x: 1, y: 2 });
    expect(acc.positions.get("db/g::b")).toEqual({ x: 3, y: 4 });
  });

  it("merges edges with namespace and counts stats", () => {
    const acc = createExportAccumulator();
    mergeExportPage(acc, page({ nodes: [pnode("a", 0, 0), pnode("b", 0, 0)] }), "db/g", "db");
    mergeExportPage(acc, page({ edges: [edge("e1", "a", "b"), edge("e1", "a", "b")] }), "db/g", "db");
    expect(acc.edges).toHaveLength(1);
    expect(acc.stats.relationship_type_counts.REFERENCES).toBe(1);
    expect(acc.stats.label_counts.Table).toBe(2);
  });

  it("keeps namespaces separate — same node id in two graphs", () => {
    const acc = createExportAccumulator();
    mergeExportPage(acc, page({ nodes: [pnode("a", 0, 0)] }), "db/g1", "db");
    mergeExportPage(acc, page({ nodes: [pnode("a", 9, 9)] }), "db/g2", "db");
    expect(acc.nodes).toHaveLength(2);
    expect(acc.positions.get("db/g2::a")).toEqual({ x: 9, y: 9 });
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react test -- graph-export-merge`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement the accumulator**

`packages/react/src/graph/graph-export-merge.ts`:

```ts
/**
 * Mutable accumulator for progressive full-graph export loading. Pages from
 * multiple (database, graph) streams merge into one node/edge set with
 * composite-id dedup (same scheme as usePensieveGraph) plus a position map.
 */
import type { GraphExportPage, GraphNode, GraphRelationship, GraphStats } from "@pensieve-ai/client";

export interface ExportAccumulator {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  positions: Map<string, { x: number; y: number }>;
  stats: GraphStats;
  seenNodes: Set<string>;
  seenEdges: Set<string>;
}

export function createExportAccumulator(): ExportAccumulator {
  return {
    nodes: [],
    edges: [],
    positions: new Map(),
    stats: {
      total_nodes: 0,
      total_relationships: 0,
      label_counts: {},
      relationship_type_counts: {},
    },
    seenNodes: new Set(),
    seenEdges: new Set(),
  };
}

/** Merge one export page into the accumulator (mutates + returns it). */
export function mergeExportPage(
  acc: ExportAccumulator,
  page: GraphExportPage,
  namespace: string,
  database: string,
): ExportAccumulator {
  for (const pn of page.nodes) {
    const key = `${namespace}::${pn.id}`;
    if (acc.seenNodes.has(key)) continue;
    acc.seenNodes.add(key);
    const { x, y, ...node } = pn;
    acc.nodes.push({ ...node, namespace, database });
    acc.positions.set(key, { x, y });
    const label = node.labels[0] ?? "Node";
    acc.stats.label_counts[label] = (acc.stats.label_counts[label] ?? 0) + 1;
  }
  for (const e of page.edges) {
    const key = `${namespace}::${e.id}`;
    if (acc.seenEdges.has(key)) continue;
    acc.seenEdges.add(key);
    acc.edges.push({ ...e, namespace, database });
    acc.stats.relationship_type_counts[e.relationship_type] =
      (acc.stats.relationship_type_counts[e.relationship_type] ?? 0) + 1;
  }
  acc.stats.total_nodes = acc.nodes.length;
  acc.stats.total_relationships = acc.edges.length;
  return acc;
}
```

- [ ] **Step 4: Run merge tests**

Run: `pnpm --filter @pensieve-ai/react test -- graph-export-merge`
Expected: PASS.

- [ ] **Step 5: Extract `useGraphCoords` and implement `useGraphExport`**

In `usePensieveGraph.ts`, lift the two discovery queries + `resolvedCoords` memo (current lines 90–129) into an exported hook in the same file — `usePensieveGraph` calls it so its behavior is unchanged:

```ts
export function useGraphCoords(args?: Pick<UsePensieveGraphArgs, "graphs" | "discover">): {
  coords: GraphCoord[];
  isLoading: boolean;
  error: unknown;
}
```

Create `packages/react/src/hooks/useGraphExport.ts`:

```ts
/**
 * useGraphExport — loads the FULL graph (all pages, all coords) with
 * server-computed positions. Pages stream into an ExportAccumulator and the
 * hook re-renders per page so the canvas fills progressively. Streams that
 * report `layout_status: "computing"` are polled until ready.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import type { LayoutAlgorithm } from "@pensieve-ai/client";
import { usePensieveClient } from "../provider/context";
import { graphKey, useGraphCoords, type UsePensieveGraphArgs } from "./usePensieveGraph";
import {
  createExportAccumulator,
  mergeExportPage,
  type ExportAccumulator,
} from "../graph/graph-export-merge";

const COMPUTING_POLL_MS = 1500;
const PAGE_SIZE = 10_000;

export interface UseGraphExportResult {
  acc: ExportAccumulator;
  /** Bumped on every merged page — memo key for downstream consumers. */
  version: number;
  coords: ReturnType<typeof useGraphCoords>["coords"];
  /** True while any stream is still computing server-side layout. */
  layoutComputing: boolean;
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  refetch: () => void;
}

export function useGraphExport(
  args: UsePensieveGraphArgs & { algorithm: LayoutAlgorithm },
): UseGraphExportResult {
  const client = usePensieveClient();
  const { coords, isLoading: discovering, error: discoveryError } = useGraphCoords(args);
  const [version, setVersion] = useState(0);
  const [layoutComputing, setLayoutComputing] = useState(false);
  const [streamsDone, setStreamsDone] = useState(0);
  const [error, setError] = useState<unknown>(null);
  const accRef = useRef<ExportAccumulator>(createExportAccumulator());
  const [generation, setGeneration] = useState(0);

  const coordsKey = coords.map((c) => graphKey(c.database, c.graph)).join(",");

  useEffect(() => {
    if (coords.length === 0) return;
    let cancelled = false;
    accRef.current = createExportAccumulator();
    setVersion((v) => v + 1);
    setStreamsDone(0);
    setError(null);

    const runStream = async (database: string, graph: string) => {
      const ns = graphKey(database, graph);
      let cursor: string | undefined;
      for (;;) {
        if (cancelled) return;
        const page = await client.withDatabase(database).graph.exportGraph({
          graph,
          realm: args.realm,
          algorithm: args.algorithm,
          cursor,
          pageSize: PAGE_SIZE,
        });
        if (cancelled) return;
        if (page.layout_status === "computing") {
          setLayoutComputing(true);
          await new Promise((r) => setTimeout(r, COMPUTING_POLL_MS));
          continue; // re-request first page
        }
        mergeExportPage(accRef.current, page, ns, database);
        setVersion((v) => v + 1);
        if (!page.next_cursor) return;
        cursor = page.next_cursor;
      }
    };

    void Promise.allSettled(
      coords.map((c) =>
        runStream(c.database, c.graph)
          .catch((e) => {
            setError((prev: unknown) => prev ?? e);
            throw e;
          })
          .finally(() => !cancelled && setStreamsDone((d) => d + 1)),
      ),
    ).then(() => !cancelled && setLayoutComputing(false));

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [coordsKey, args.realm, args.algorithm, generation]);

  const refetch = useCallback(() => setGeneration((g) => g + 1), []);

  return {
    acc: accRef.current,
    version,
    coords,
    layoutComputing,
    isLoading: discovering || (coords.length > 0 && streamsDone < coords.length && version <= 1),
    isError: !discovering && Boolean(discoveryError ?? error) && accRef.current.nodes.length === 0,
    error: error ?? discoveryError,
    refetch,
  };
}
```

- [ ] **Step 6: Verify**

Run: `pnpm --filter @pensieve-ai/react test && pnpm --filter @pensieve-ai/react typecheck`
Expected: PASS — existing `usePensieveGraph` consumers unaffected (its public return is unchanged).

- [ ] **Step 7: Commit**

```bash
git add packages/react/src/graph/graph-export-merge.ts packages/react/src/graph/graph-export-merge.test.ts packages/react/src/hooks/useGraphExport.ts packages/react/src/hooks/usePensieveGraph.ts
git commit -m "feat(react): chunked full-graph export loading with positions + progressive merge"
```

---

## Phase C — Sigma renderer

### Task 6: dependencies + display logic (LOD, quiet/loud)

Pure display functions first — they're the testable heart of the renderer.

**Files:**
- Modify: `packages/react/package.json` (deps)
- Create: `packages/react/src/graph/graph-display.ts`
- Test: `packages/react/src/graph/graph-display.test.ts`

- [ ] **Step 1: Install dependencies**

```bash
pnpm --filter @pensieve-ai/react add sigma@^3.0.0 graphology@^0.26.0 graphology-types@^0.24.8 @sigma/edge-curve@^3.0.0 @sigma/node-image@^3.0.0
```

Expected: lockfile updated, install clean.

- [ ] **Step 2: Write the failing display tests**

`packages/react/src/graph/graph-display.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import {
  FAR_RATIO,
  NEAR_RATIO,
  edgeDisplay,
  lodTier,
  nodeDisplay,
  type DisplayCtx,
} from "./graph-display";

const baseCtx = (over: Partial<DisplayCtx> = {}): DisplayCtx => ({
  tier: "mid",
  focusId: null,
  neighborhood: null,
  searchMatches: null,
  relTypeFilter: null,
  showEdgeLabels: false,
  isDark: true,
  focusModeIds: null,
  ...over,
});

describe("lodTier", () => {
  it("maps camera ratio to tiers", () => {
    expect(lodTier(FAR_RATIO + 1)).toBe("far");
    expect(lodTier((FAR_RATIO + NEAR_RATIO) / 2)).toBe("mid");
    expect(lodTier(NEAR_RATIO / 2)).toBe("near");
  });
});

describe("nodeDisplay (quiet/loud)", () => {
  const attrs = { label: "svc", color: "#22d3ee", size: 6, image: undefined };

  it("at rest: keeps color, hides label when far", () => {
    const d = nodeDisplay("a", attrs, baseCtx({ tier: "far" }));
    expect(d.label).toBeNull();
    expect(d.dimmed).toBe(false);
  });

  it("dims nodes outside the focus neighborhood", () => {
    const ctx = baseCtx({ focusId: "x", neighborhood: new Set(["x", "y"]) });
    expect(nodeDisplay("a", attrs, ctx).dimmed).toBe(true);
    expect(nodeDisplay("y", attrs, ctx).dimmed).toBe(false);
    expect(nodeDisplay("x", attrs, ctx).highlighted).toBe(true);
  });

  it("search matches stay lit and labeled even when far", () => {
    const ctx = baseCtx({ tier: "far", searchMatches: new Set(["a"]) });
    const d = nodeDisplay("a", attrs, ctx);
    expect(d.dimmed).toBe(false);
    expect(d.label).toBe("svc");
  });

  it("focus mode hides everything outside the focus set", () => {
    const ctx = baseCtx({ focusModeIds: new Set(["a"]) });
    expect(nodeDisplay("b", attrs, ctx).hidden).toBe(true);
    expect(nodeDisplay("a", attrs, ctx).hidden).toBe(false);
  });
});

describe("edgeDisplay (quiet/loud)", () => {
  const attrs = { color: "#f59e0b", size: 1, relType: "DEPENDS_ON" };

  it("at rest: low alpha, no label, thin", () => {
    const d = edgeDisplay("e1", attrs, "a", "b", baseCtx());
    expect(d.alpha).toBeLessThan(0.5);
    expect(d.label).toBeNull();
  });

  it("loud when incident to focus: bold + labeled", () => {
    const ctx = baseCtx({ focusId: "a", neighborhood: new Set(["a", "b"]) });
    const d = edgeDisplay("e1", attrs, "a", "b", ctx);
    expect(d.alpha).toBeGreaterThan(0.85);
    expect(d.size).toBeGreaterThan(attrs.size);
    expect(d.label).toBe("DEPENDS_ON");
  });

  it("near-mute when focus exists elsewhere", () => {
    const ctx = baseCtx({ focusId: "x", neighborhood: new Set(["x"]) });
    expect(edgeDisplay("e1", attrs, "a", "b", ctx).alpha).toBeLessThanOrEqual(0.08);
  });

  it("relationship-type isolation filter", () => {
    const ctx = baseCtx({ relTypeFilter: "CONTAINS" });
    expect(edgeDisplay("e1", attrs, "a", "b", ctx).alpha).toBeLessThanOrEqual(0.06);
    const d = edgeDisplay("e1", { ...attrs, relType: "CONTAINS" }, "a", "b", ctx);
    expect(d.alpha).toBeGreaterThan(0.85);
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react test -- graph-display`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement**

`packages/react/src/graph/graph-display.ts`:

```ts
/**
 * Pure display logic for the Sigma renderer: zoom LOD tiers and the
 * quiet-arrows/loud-focus styling contract. Sigma's node/edge reducers call
 * these every frame, so everything here must be allocation-light.
 *
 * Tiers (sigma camera ratio grows when zooming OUT; ratio 1 ≈ fitted view):
 *   far  — whole-galaxy view: dots + hulls, landmark labels only
 *   mid  — arrows + icons visible, focused labels
 *   near — all labels, edge labels on focus
 */

export type LodTier = "far" | "mid" | "near";

export const FAR_RATIO = 0.8;
export const NEAR_RATIO = 0.25;

export function lodTier(cameraRatio: number): LodTier {
  if (cameraRatio >= FAR_RATIO) return "far";
  if (cameraRatio >= NEAR_RATIO) return "mid";
  return "near";
}

export interface DisplayCtx {
  tier: LodTier;
  /** Hovered or selected composite id (hover wins) — the "loud" center. */
  focusId: string | null;
  /** focusId + its 1-hop neighbors. Null when no focus. */
  neighborhood: Set<string> | null;
  /** Composite ids matching the active search. Null when no search. */
  searchMatches: Set<string> | null;
  relTypeFilter: string | null;
  showEdgeLabels: boolean;
  isDark: boolean;
  /** Focus-neighborhood isolation (double-click): only these ids render. */
  focusModeIds: Set<string> | null;
}

export interface NodeDisplay {
  hidden: boolean;
  dimmed: boolean;
  highlighted: boolean;
  label: string | null;
}

export function nodeDisplay(
  id: string,
  attrs: { label: string; size: number },
  ctx: DisplayCtx,
): NodeDisplay {
  if (ctx.focusModeIds && !ctx.focusModeIds.has(id)) {
    return { hidden: true, dimmed: false, highlighted: false, label: null };
  }
  const isMatch = ctx.searchMatches?.has(id) ?? false;
  const inHood = ctx.neighborhood?.has(id) ?? false;
  const hasFocus = ctx.focusId !== null;
  const hasSearch = ctx.searchMatches !== null;
  const dimmed = (hasFocus && !inHood) || (hasSearch && !isMatch);
  const highlighted = id === ctx.focusId || isMatch;
  // Labels: always for highlighted/neighborhood; by tier otherwise.
  const label =
    highlighted || (inHood && ctx.tier !== "far")
      ? attrs.label
      : dimmed
        ? null
        : ctx.tier === "near"
          ? attrs.label
          : ctx.tier === "mid" && attrs.size >= 8 // landmark-ish nodes
            ? attrs.label
            : null;
  return { hidden: false, dimmed, highlighted, label };
}

export interface EdgeDisplay {
  hidden: boolean;
  alpha: number;
  size: number;
  label: string | null;
  /** True → bold arrowhead (loud); false → small quiet arrowhead. */
  loud: boolean;
}

export function edgeDisplay(
  _id: string,
  attrs: { size: number; relType: string },
  source: string,
  target: string,
  ctx: DisplayCtx,
): EdgeDisplay {
  if (ctx.focusModeIds && !(ctx.focusModeIds.has(source) && ctx.focusModeIds.has(target))) {
    return { hidden: true, alpha: 0, size: 0, label: null, loud: false };
  }
  if (ctx.relTypeFilter) {
    if (attrs.relType !== ctx.relTypeFilter) {
      return { hidden: false, alpha: 0.04, size: attrs.size * 0.8, label: null, loud: false };
    }
    return {
      hidden: false,
      alpha: 0.95,
      size: attrs.size * 1.6,
      label: ctx.tier === "near" ? attrs.relType : null,
      loud: true,
    };
  }
  const hasFocus = ctx.focusId !== null;
  const incident =
    hasFocus && (source === ctx.focusId || target === ctx.focusId);
  if (incident) {
    return {
      hidden: false,
      alpha: 0.95,
      size: Math.max(attrs.size * 1.8, 2),
      label: ctx.tier === "far" ? null : attrs.relType,
      loud: true,
    };
  }
  if (hasFocus) {
    return { hidden: false, alpha: 0.05, size: attrs.size * 0.8, label: null, loud: false };
  }
  // At rest: quiet — visible color, small arrowhead, optional global labels.
  return {
    hidden: false,
    alpha: ctx.tier === "far" ? 0.22 : ctx.isDark ? 0.38 : 0.5,
    size: attrs.size,
    label: ctx.showEdgeLabels && ctx.tier === "near" ? attrs.relType : null,
    loud: false,
  };
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `pnpm --filter @pensieve-ai/react test -- graph-display`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/react/package.json pnpm-lock.yaml packages/react/src/graph/graph-display.ts packages/react/src/graph/graph-display.test.ts
git commit -m "feat(react): sigma deps + pure LOD/quiet-loud display logic"
```

---

### Task 7: SigmaCanvas component

**Files:**
- Create: `packages/react/src/graph/SigmaCanvas.tsx`
- Modify: `packages/react/src/graph/graph-icons.tsx` (add `getIconDataUrl` — extract the data-URL construction out of `getIconImage`, graph-icons.tsx:410-426)
- Test: `packages/react/src/graph/SigmaCanvas.test.tsx` (graph-building logic only — sigma needs WebGL, unavailable in jsdom)

- [ ] **Step 1: Add `getIconDataUrl` to graph-icons.tsx**

Refactor `getIconImage` so both share one builder:

```ts
/** Data-URL SVG for an icon at a color — used as sigma's node `image`. */
export function getIconDataUrl(icon: ResolvedIcon, color: string): string {
  const svg = renderToStaticMarkup(
    createElement(icon.Comp, { size: 40, color, stroke: color, strokeWidth: icon.brand ? 1.8 : 2 }),
  );
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`;
}
```

and change `getIconImage`'s `img.src = ...` line to `img.src = getIconDataUrl(icon, color);`.

- [ ] **Step 2: Write the failing graph-builder test**

Export the builder separately so it tests without WebGL. `SigmaCanvas.test.tsx`:

```tsx
import { describe, expect, it } from "vitest";
import { buildGraphologyGraph } from "./SigmaCanvas";
import { createExportAccumulator, mergeExportPage } from "./graph-export-merge";

const meta = { created_at: "", updated_at: "", realm: "default" };

describe("buildGraphologyGraph", () => {
  it("creates positioned, sized, namespaced nodes and directed edges", () => {
    const acc = createExportAccumulator();
    mergeExportPage(
      acc,
      {
        layout_status: "ready", layout_id: "L", total_nodes: 2, total_edges: 1,
        nodes: [
          { id: "a", labels: ["Service"], properties: { name: "api" }, metadata: meta, x: 5, y: 6 },
          { id: "b", labels: ["Table"], properties: {}, metadata: meta, x: 7, y: 8 },
        ],
        edges: [{ id: "e1", source_id: "a", target_id: "b", relationship_type: "DEPENDS_ON", properties: {} }],
        next_cursor: null,
      },
      "db/g",
      "db",
    );
    const g = buildGraphologyGraph(acc.nodes, acc.edges, acc.positions, {
      sizeByDegree: true,
      hiddenLabels: [],
      activeNamespaces: new Set(["db/g"]),
    });
    expect(g.order).toBe(2);
    expect(g.size).toBe(1);
    expect(g.getNodeAttribute("db/g::a", "x")).toBe(5);
    expect(g.getNodeAttribute("db/g::a", "label")).toBe("api");
    expect(g.getEdgeAttribute("db/g::e1", "relType")).toBe("DEPENDS_ON");
    // hub sizing: degree-1 nodes get base size
    expect(g.getNodeAttribute("db/g::b", "size")).toBeGreaterThan(0);
  });

  it("filters hidden labels and namespaces", () => {
    const acc = createExportAccumulator();
    mergeExportPage(
      acc,
      {
        layout_status: "ready", layout_id: "L", total_nodes: 1, total_edges: 0,
        nodes: [{ id: "a", labels: ["Secret"], properties: {}, metadata: meta, x: 0, y: 0 }],
        edges: [], next_cursor: null,
      },
      "db/g",
      "db",
    );
    const g = buildGraphologyGraph(acc.nodes, acc.edges, acc.positions, {
      sizeByDegree: true,
      hiddenLabels: ["Secret"],
      activeNamespaces: new Set(["db/g"]),
    });
    expect(g.order).toBe(0);
  });
});
```

- [ ] **Step 3: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react test -- SigmaCanvas`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement SigmaCanvas**

`packages/react/src/graph/SigmaCanvas.tsx` — the component owns: graphology build, sigma instance, reducers wired to `graph-display.ts`, events, camera fly-to. Complete implementation:

```tsx
/**
 * SigmaCanvas — WebGL renderer for the full graph. Replaces GraphCanvas on
 * WebGL-capable browsers. Nodes/edges/positions come fully loaded from
 * useGraphExport; this component renders ALL of them and lets the
 * quiet/loud reducers + LOD carry legibility.
 */
import { useEffect, useMemo, useRef } from "react";
import Graph from "graphology";
import Sigma from "sigma";
import { createNodeImageProgram } from "@sigma/node-image";
import { EdgeCurvedArrowProgram } from "@sigma/edge-curve";
import { EdgeArrowProgram } from "sigma/rendering";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { usePensieveContext } from "../provider/context";
import { useGraphStore } from "./graph-store";
import { edgeDisplay, lodTier, nodeDisplay, type DisplayCtx } from "./graph-display";
import { getRelationshipFamilyColor, alpha as withAlpha, radiusForDegree } from "./graph-style";
import { getIconDataUrl, resolveGraphIcon, resolveNodeColor } from "./graph-icons";

const keyOf = (n: { id: string; namespace?: string }) => `${n.namespace ?? ""}::${n.id}`;
const edgeSrcKey = (e: GraphRelationship) => `${e.namespace ?? ""}::${e.source_id}`;
const edgeDstKey = (e: GraphRelationship) =>
  `${(e.properties?.target_namespace as string | undefined) ?? e.namespace ?? ""}::${e.target_id}`;

export interface BuildOptions {
  sizeByDegree: boolean;
  hiddenLabels: string[];
  activeNamespaces: Set<string>;
}

/** Build the graphology instance — exported for tests. */
export function buildGraphologyGraph(
  nodes: GraphNode[],
  edges: GraphRelationship[],
  positions: Map<string, { x: number; y: number }>,
  opts: BuildOptions,
): Graph {
  const g = new Graph({ multi: true, type: "directed" });
  const degree = new Map<string, number>();
  for (const e of edges) {
    degree.set(edgeSrcKey(e), (degree.get(edgeSrcKey(e)) ?? 0) + 1);
    degree.set(edgeDstKey(e), (degree.get(edgeDstKey(e)) ?? 0) + 1);
  }
  const degs = [...degree.values()].sort((a, b) => a - b);
  const capDeg = degs.length ? degs[Math.floor(degs.length * 0.95)] : 1;

  for (const n of nodes) {
    if (!opts.activeNamespaces.has(n.namespace ?? "")) continue;
    if (opts.hiddenLabels.includes(n.labels[0] ?? "")) continue;
    const key = keyOf(n);
    const pos = positions.get(key) ?? { x: 0, y: 0 };
    const color = resolveNodeColor(n.labels, n.properties);
    const icon = resolveGraphIcon(n.labels, n.properties);
    const name =
      (n.properties?.name as string) || (n.properties?.title as string) || n.id;
    g.addNode(key, {
      x: pos.x,
      y: pos.y,
      size: radiusForDegree(degree.get(key) ?? 0, capDeg, opts.sizeByDegree),
      color,
      label: name,
      image: icon ? getIconDataUrl(icon, "#ffffff") : undefined,
      type: icon ? "image" : "circle",
      nodeLabel: n.labels[0] ?? "Node",
    });
  }
  for (const e of edges) {
    const s = edgeSrcKey(e);
    const t = edgeDstKey(e);
    if (!g.hasNode(s) || !g.hasNode(t)) continue;
    const ekey = `${e.namespace ?? ""}::${e.id}`;
    if (g.hasEdge(ekey)) continue;
    g.addDirectedEdgeWithKey(ekey, s, t, {
      size: 1,
      relType: e.relationship_type,
      familyColor: getRelationshipFamilyColor(e.relationship_type),
    });
  }
  return g;
}

export interface SigmaCanvasProps {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  positions: Map<string, { x: number; y: number }>;
  /** Bump to rebuild the graph (export accumulator version). */
  version: number;
  activeNamespaces: Set<string>;
  onNodeClick: (compositeId: string) => void;
  onNodeHover: (compositeId: string | null) => void;
  onNodeDoubleClick: (compositeId: string) => void;
  /** Receives the live sigma instance (for minimap / fly-to / keyboard). */
  onSigmaReady?: (sigma: Sigma) => void;
}

export function SigmaCanvas(props: SigmaCanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const sigmaRef = useRef<Sigma | null>(null);
  const { isDark } = usePensieveContext();

  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const hoveredNodeId = useGraphStore((s) => s.hoveredNodeId);
  const focusSeq = useGraphStore((s) => s.focusSeq);
  const searchQuery = useGraphStore((s) => s.searchQuery);
  const relTypeFilter = useGraphStore((s) => s.relTypeFilter);
  const hiddenLabels = useGraphStore((s) => s.hiddenLabels);
  const showEdgeLabels = useGraphStore((s) => s.showEdgeLabels);
  const sizeByDegree = useGraphStore((s) => s.sizeByDegree);
  const curvedEdges = useGraphStore((s) => s.curvedEdges);
  const focusModeId = useGraphStore((s) => s.focusModeId);

  const graph = useMemo(
    () =>
      buildGraphologyGraph(props.nodes, props.edges, props.positions, {
        sizeByDegree,
        hiddenLabels,
        activeNamespaces: props.activeNamespaces,
      }),
    // version covers nodes/edges/positions (accumulator is mutable)
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [props.version, sizeByDegree, hiddenLabels, props.activeNamespaces],
  );

  // Display context, rebuilt when interaction state changes.
  const ctxRef = useRef<DisplayCtx>(null as unknown as DisplayCtx);
  ctxRef.current = useMemo<DisplayCtx>(() => {
    const focusId = hoveredNodeId ?? selectedNodeId;
    let neighborhood: Set<string> | null = null;
    if (focusId && graph.hasNode(focusId)) {
      neighborhood = new Set([focusId]);
      for (const nb of graph.neighbors(focusId)) neighborhood.add(nb);
    }
    let searchMatches: Set<string> | null = null;
    if (searchQuery.trim()) {
      const q = searchQuery.trim().toLowerCase();
      searchMatches = new Set();
      graph.forEachNode((id, attrs) => {
        if (String(attrs.label).toLowerCase().includes(q) || id.toLowerCase().includes(q)) {
          searchMatches!.add(id);
        }
      });
    }
    let focusModeIds: Set<string> | null = null;
    if (focusModeId && graph.hasNode(focusModeId)) {
      focusModeIds = new Set([focusModeId]);
      for (const nb of graph.neighbors(focusModeId)) {
        focusModeIds.add(nb);
        for (const nb2 of graph.neighbors(nb)) focusModeIds.add(nb2);
      }
    }
    return {
      tier: lodTier(sigmaRef.current?.getCamera().ratio ?? 1),
      focusId,
      neighborhood,
      searchMatches,
      relTypeFilter,
      showEdgeLabels,
      isDark,
      focusModeIds,
    };
  }, [graph, hoveredNodeId, selectedNodeId, searchQuery, relTypeFilter, showEdgeLabels, isDark, focusModeId]);

  // Instantiate sigma once per graph.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const sigma = new Sigma(graph, container, {
      defaultEdgeType: curvedEdges ? "curvedArrow" : "straightArrow",
      edgeProgramClasses: {
        straightArrow: EdgeArrowProgram,
        curvedArrow: EdgeCurvedArrowProgram,
      },
      nodeProgramClasses: { image: createNodeImageProgram({ keepWithinCircle: true }) },
      renderEdgeLabels: true,
      labelColor: { color: isDark ? "#cbd5e1" : "#334155" },
      edgeLabelColor: { color: isDark ? "#94a3b8" : "#64748b" },
      labelFont: "IBM Plex Sans, sans-serif",
      edgeLabelFont: "JetBrains Mono, monospace",
      labelSize: 11,
      edgeLabelSize: 9,
      minCameraRatio: 0.02,
      maxCameraRatio: 4,
      labelRenderedSizeThreshold: 0, // reducers own label visibility
      nodeReducer: (id, attrs) => {
        const d = nodeDisplay(id, attrs as { label: string; size: number }, ctxRef.current);
        return {
          ...attrs,
          hidden: d.hidden,
          label: d.label,
          color: d.dimmed ? withAlpha(attrs.color as string, 0.15) : (attrs.color as string),
          size: d.highlighted ? (attrs.size as number) * 1.25 : (attrs.size as number),
          zIndex: d.highlighted ? 2 : d.dimmed ? 0 : 1,
          highlighted: d.highlighted,
        };
      },
      edgeReducer: (id, attrs) => {
        const [src, tgt] = sigma.getGraph().extremities(id);
        const d = edgeDisplay(
          id,
          attrs as { size: number; relType: string },
          src,
          tgt,
          ctxRef.current,
        );
        return {
          ...attrs,
          hidden: d.hidden,
          color: withAlpha(attrs.familyColor as string, d.alpha),
          size: d.size,
          label: d.label,
          zIndex: d.loud ? 2 : 0,
        };
      },
    });
    sigmaRef.current = sigma;
    props.onSigmaReady?.(sigma);

    sigma.on("clickNode", ({ node }) => props.onNodeClick(node));
    sigma.on("doubleClickNode", ({ node, event }) => {
      event.preventSigmaDefault();
      props.onNodeDoubleClick(node);
    });
    sigma.on("clickStage", () => props.onNodeClick(""));
    sigma.on("enterNode", ({ node }) => props.onNodeHover(node));
    sigma.on("leaveNode", () => props.onNodeHover(null));
    // LOD: refresh on zoom so reducers re-run with the new tier.
    sigma.getCamera().on("updated", () => sigma.refresh({ skipIndexation: true }));

    return () => {
      sigmaRef.current = null;
      sigma.kill();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, curvedEdges, isDark]);

  // Re-run reducers when interaction context changes.
  useEffect(() => {
    sigmaRef.current?.refresh({ skipIndexation: true });
  }, [hoveredNodeId, selectedNodeId, searchQuery, relTypeFilter, showEdgeLabels, focusModeId]);

  // Fly-to on deep-link / command-bar focus (focusSeq bumps).
  useEffect(() => {
    const sigma = sigmaRef.current;
    if (!sigma || !selectedNodeId || focusSeq === 0) return;
    if (!sigma.getGraph().hasNode(selectedNodeId)) return;
    const pos = sigma.getNodeDisplayData(selectedNodeId);
    if (!pos) return;
    sigma.getCamera().animate({ x: pos.x, y: pos.y, ratio: 0.12 }, { duration: 600 });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusSeq]);

  return <div ref={containerRef} className="pv-absolute pv-inset-0" />;
}
```

Note on `radiusForDegree`: it exists in `graph-style.ts:100` — verify its signature matches `(degree, capDeg, sizeByDegree)` and adapt the call if not.

- [ ] **Step 5: Run tests + typecheck**

Run: `pnpm --filter @pensieve-ai/react test -- SigmaCanvas && pnpm --filter @pensieve-ai/react typecheck`
Expected: builder tests PASS, typecheck clean. (Sigma's reducer/setting types moved around in v3 minors — fix signatures per the installed version's `.d.ts`, not by casting to `any`.)

- [ ] **Step 6: Commit**

```bash
git add packages/react/src/graph/SigmaCanvas.tsx packages/react/src/graph/SigmaCanvas.test.tsx packages/react/src/graph/graph-icons.tsx
git commit -m "feat(react): SigmaCanvas WebGL renderer with quiet/loud reducers and LOD"
```

---

### Task 8: community hulls layer

**Files:**
- Create: `packages/react/src/graph/HullsLayer.tsx`
- Test: covered by existing `graph-community.ts` logic (already tested behavior); HullsLayer is draw-only.

- [ ] **Step 1: Implement**

```tsx
/**
 * HullsLayer — translucent community blobs under the sigma canvases.
 * Communities come from detectCommunities (label propagation); hull points
 * are projected with sigma.graphToViewport on every render so they track
 * camera moves exactly.
 */
import { useEffect, useMemo, useRef } from "react";
import type Sigma from "sigma";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { convexHull, detectCommunities, padHull } from "./graph-community";
import { getLabelColor } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";

export function HullsLayer({
  sigma,
  nodes,
  edges,
  version,
}: {
  sigma: Sigma | null;
  nodes: GraphNode[];
  edges: GraphRelationship[];
  version: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const communityHulls = useGraphStore((s) => s.communityHulls);

  const communities = useMemo(
    () => (communityHulls ? detectCommunities(nodes, edges) : null),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [version, communityHulls],
  );

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !sigma) return;
    const draw = () => {
      const { width, height } = sigma.getDimensions();
      canvas.width = width * window.devicePixelRatio;
      canvas.height = height * window.devicePixelRatio;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      const ctx = canvas.getContext("2d")!;
      ctx.setTransform(window.devicePixelRatio, 0, 0, window.devicePixelRatio, 0, 0);
      ctx.clearRect(0, 0, width, height);
      if (!communities) return;
      const graph = sigma.getGraph();
      for (const [communityId, memberIds] of communities) {
        if (memberIds.length < 3) continue;
        const pts = memberIds
          .filter((id) => graph.hasNode(id))
          .map((id) => {
            const a = graph.getNodeAttributes(id);
            return sigma.graphToViewport({ x: a.x as number, y: a.y as number });
          });
        if (pts.length < 3) continue;
        const hull = padHull(convexHull(pts), 26);
        ctx.beginPath();
        hull.forEach((p, i) => (i === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
        ctx.closePath();
        const color = getLabelColor(String(communityId));
        ctx.fillStyle = `${color}14`;
        ctx.strokeStyle = `${color}30`;
        ctx.fill();
        ctx.stroke();
      }
    };
    draw();
    sigma.on("afterRender", draw);
    return () => {
      sigma.off("afterRender", draw);
    };
  }, [sigma, communities]);

  if (!communityHulls) return null;
  return <canvas ref={canvasRef} className="pv-pointer-events-none pv-absolute pv-inset-0" />;
}
```

Check `detectCommunities`'s actual return shape at `graph-community.ts:20` (it may return a `Map<community, string[]>` keyed differently or take composite-id'd nodes) and adapt the iteration — the drawing contract above is what matters.

- [ ] **Step 2: Typecheck + commit**

Run: `pnpm --filter @pensieve-ai/react typecheck`

```bash
git add packages/react/src/graph/HullsLayer.tsx
git commit -m "feat(react): community hull layer tracking the sigma camera"
```

---

## Phase D — Chrome & navigation

### Task 9: store extensions (trail, focus mode, command bar)

**Files:**
- Modify: `packages/react/src/graph/graph-store.ts`
- Test: `packages/react/src/graph/graph-store.test.ts` (create)

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it } from "vitest";
import { createGraphStore } from "./graph-store";

describe("trail", () => {
  it("appends visited nodes, dedups consecutive, caps at 20", () => {
    const s = createGraphStore();
    s.getState().pushTrail("db/g::a");
    s.getState().pushTrail("db/g::a");
    s.getState().pushTrail("db/g::b");
    expect(s.getState().trail).toEqual(["db/g::a", "db/g::b"]);
    for (let i = 0; i < 30; i++) s.getState().pushTrail(`db/g::n${i}`);
    expect(s.getState().trail.length).toBe(20);
  });

  it("jumpTrail truncates after the target and selects it", () => {
    const s = createGraphStore();
    ["a", "b", "c"].forEach((id) => s.getState().pushTrail(id));
    s.getState().jumpTrail(0);
    expect(s.getState().trail).toEqual(["a"]);
    expect(s.getState().selectedNodeId).toBe("a");
    expect(s.getState().focusSeq).toBeGreaterThan(0); // triggers fly-to
  });
});

describe("focus mode + command bar", () => {
  it("focusModeId set/clear", () => {
    const s = createGraphStore();
    s.getState().setFocusMode("db/g::a");
    expect(s.getState().focusModeId).toBe("db/g::a");
    s.getState().setFocusMode(null);
    expect(s.getState().focusModeId).toBeNull();
  });

  it("commandBarOpen toggles", () => {
    const s = createGraphStore();
    s.getState().setCommandBarOpen(true);
    expect(s.getState().commandBarOpen).toBe(true);
  });

  it("setGraph resets focus mode but keeps the trail", () => {
    const s = createGraphStore();
    s.getState().pushTrail("x");
    s.getState().setFocusMode("x");
    s.getState().setGraph("db/g");
    expect(s.getState().focusModeId).toBeNull();
    expect(s.getState().trail).toEqual(["x"]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react test -- graph-store`
Expected: FAIL — `pushTrail is not a function`.

- [ ] **Step 3: Implement**

Add to `GraphStoreState` (graph-store.ts):

```ts
  /** Visited-node breadcrumb trail (composite ids, newest last, max 20). */
  trail: string[];
  /** Focus-neighborhood isolation root (double-click). Null = off. */
  focusModeId: string | null;
  commandBarOpen: boolean;

  pushTrail(id: string): void;
  jumpTrail(index: number): void;
  clearTrail(): void;
  setFocusMode(id: string | null): void;
  setCommandBarOpen(v: boolean): void;
```

To `initialGraphState`: `trail: [] as string[], focusModeId: null as string | null, commandBarOpen: false,`

To the factory:

```ts
    pushTrail: (id) =>
      set((s) => {
        if (s.trail[s.trail.length - 1] === id) return {};
        return { trail: [...s.trail, id].slice(-20) };
      }),
    jumpTrail: (index) =>
      set((s) => {
        const target = s.trail[index];
        if (!target) return {};
        return {
          trail: s.trail.slice(0, index + 1),
          selectedNodeId: target,
          focusSeq: s.focusSeq + 1,
        };
      }),
    clearTrail: () => set({ trail: [] }),
    setFocusMode: (id) => set({ focusModeId: id }),
    setCommandBarOpen: (v) => set({ commandBarOpen: v }),
```

And in the existing `setGraph` action add `focusModeId: null` to the reset object.

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @pensieve-ai/react test -- graph-store`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/react/src/graph/graph-store.ts packages/react/src/graph/graph-store.test.ts
git commit -m "feat(react): graph store trail, focus mode, command bar state"
```

---

### Task 10: keyboard edge-walking

**Files:**
- Create: `packages/react/src/graph/graph-walk.ts` (pure neighbor sort) + `packages/react/src/graph/useKeyboardWalk.ts`
- Test: `packages/react/src/graph/graph-walk.test.ts`

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it } from "vitest";
import Graph from "graphology";
import { sortedNeighbors } from "./graph-walk";

const g = new Graph({ multi: true, type: "directed" });
["hub", "a", "b", "c"].forEach((n) => g.addNode(n, { x: 0, y: 0, size: 1 }));
g.addEdge("hub", "a", { relType: "DEPENDS_ON" });
g.addEdge("hub", "b", { relType: "CONTAINS" });
g.addEdge("c", "hub", { relType: "CONTAINS" });
g.addEdge("a", "b", { relType: "CONTAINS" }); // gives b degree 2 vs a's 2 vs c's 1

describe("sortedNeighbors", () => {
  it("sorts by relationship type, then degree desc, then id", () => {
    const result = sortedNeighbors(g, "hub");
    // CONTAINS (b: deg2, c: deg1) before DEPENDS_ON (a)
    expect(result.map((r) => r.nodeId)).toEqual(["b", "c", "a"]);
    expect(result[0].relType).toBe("CONTAINS");
    expect(result[2].direction).toBe("out");
    expect(result[1].direction).toBe("in");
  });

  it("empty for unknown node", () => {
    expect(sortedNeighbors(g, "ghost")).toEqual([]);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react test -- graph-walk`
Expected: FAIL.

- [ ] **Step 3: Implement**

`graph-walk.ts`:

```ts
/** Pure neighbor ordering for keyboard edge-walking: group by relationship
 * type (alphabetical), then degree desc (important nodes first), then id. */
import type Graph from "graphology";

export interface WalkCandidate {
  nodeId: string;
  relType: string;
  direction: "in" | "out";
  edgeId: string;
}

export function sortedNeighbors(graph: Graph, nodeId: string): WalkCandidate[] {
  if (!graph.hasNode(nodeId)) return [];
  const out: WalkCandidate[] = [];
  graph.forEachOutEdge(nodeId, (edge, attrs, _s, target) => {
    out.push({ nodeId: target, relType: String(attrs.relType), direction: "out", edgeId: edge });
  });
  graph.forEachInEdge(nodeId, (edge, attrs, source) => {
    out.push({ nodeId: source, relType: String(attrs.relType), direction: "in", edgeId: edge });
  });
  return out.sort(
    (a, b) =>
      a.relType.localeCompare(b.relType) ||
      graph.degree(b.nodeId) - graph.degree(a.nodeId) ||
      a.nodeId.localeCompare(b.nodeId),
  );
}
```

`useKeyboardWalk.ts`:

```ts
/**
 * Keyboard edge-walking: with a node selected, Tab / ArrowRight cycles its
 * neighbors (highlighting the candidate via hoverNode), Shift+Tab / ArrowLeft
 * cycles backwards, Enter selects + flies to the candidate, Esc steps back
 * along the trail. Inactive while the command bar is open or focus is in an
 * input.
 */
import { useEffect, useRef } from "react";
import type Graph from "graphology";
import { useGraphStore } from "./graph-store";
import { sortedNeighbors } from "./graph-walk";

export function useKeyboardWalk(graphRef: React.RefObject<Graph | null>) {
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const commandBarOpen = useGraphStore((s) => s.commandBarOpen);
  const hoverNode = useGraphStore((s) => s.hoverNode);
  const focusNode = useGraphStore((s) => s.focusNode);
  const pushTrail = useGraphStore((s) => s.pushTrail);
  const jumpTrail = useGraphStore((s) => s.jumpTrail);
  const trail = useGraphStore((s) => s.trail);
  const idx = useRef(-1);

  useEffect(() => {
    idx.current = -1; // reset cycle on selection change
  }, [selectedNodeId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (commandBarOpen) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      const graph = graphRef.current;
      if (!graph || !selectedNodeId) return;

      if (e.key === "Tab" || e.key === "ArrowRight" || e.key === "ArrowLeft") {
        e.preventDefault();
        const candidates = sortedNeighbors(graph, selectedNodeId);
        if (candidates.length === 0) return;
        const dir = e.key === "ArrowLeft" || (e.key === "Tab" && e.shiftKey) ? -1 : 1;
        idx.current = (idx.current + dir + candidates.length) % candidates.length;
        hoverNode(candidates[idx.current].nodeId); // lights up candidate + edge
      } else if (e.key === "Enter" && idx.current >= 0) {
        e.preventDefault();
        const candidates = sortedNeighbors(graph, selectedNodeId);
        const target = candidates[idx.current]?.nodeId;
        if (target) {
          hoverNode(null);
          pushTrail(target);
          focusNode(target); // selects + flies (focusSeq)
        }
      } else if (e.key === "Escape" && trail.length > 1) {
        jumpTrail(trail.length - 2);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [graphRef, selectedNodeId, commandBarOpen, trail, hoverNode, focusNode, pushTrail, jumpTrail]);
}
```

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @pensieve-ai/react test -- graph-walk && pnpm --filter @pensieve-ai/react typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/react/src/graph/graph-walk.ts packages/react/src/graph/graph-walk.test.ts packages/react/src/graph/useKeyboardWalk.ts
git commit -m "feat(react): keyboard edge-walking with sorted neighbor cycling"
```

---

### Task 11: ⌘K command bar

**Files:**
- Create: `packages/react/src/graph/CommandBar.tsx`
- Test: `packages/react/src/graph/CommandBar.test.tsx` (render + filter behavior with @testing-library/react, same harness as `GraphSidebar.test.tsx`)

- [ ] **Step 1: Write the failing test**

```tsx
import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import Graph from "graphology";
import { CommandBar } from "./CommandBar";
import { GraphStoreContext, createGraphStore } from "./graph-store";

function setup() {
  const store = createGraphStore({ commandBarOpen: true });
  const graph = new Graph({ multi: true, type: "directed" });
  graph.addNode("db/g::a", { label: "payment-svc", nodeLabel: "Service", x: 0, y: 0, size: 1 });
  graph.addNode("db/g::b", { label: "orders-table", nodeLabel: "Table", x: 0, y: 0, size: 1 });
  const utils = render(
    <GraphStoreContext.Provider value={store}>
      <CommandBar graphRef={{ current: graph }} />
    </GraphStoreContext.Provider>,
  );
  return { store, ...utils };
}

describe("CommandBar", () => {
  it("filters nodes by text and selects on click", () => {
    const { store } = setup();
    fireEvent.change(screen.getByPlaceholderText(/search/i), { target: { value: "payment" } });
    expect(screen.getByText("payment-svc")).toBeInTheDocument();
    expect(screen.queryByText("orders-table")).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("payment-svc"));
    expect(store.getState().selectedNodeId).toBe("db/g::a");
    expect(store.getState().commandBarOpen).toBe(false);
    expect(store.getState().trail).toEqual(["db/g::a"]);
  });

  it("closes on Escape", () => {
    const { store } = setup();
    fireEvent.keyDown(screen.getByPlaceholderText(/search/i), { key: "Escape" });
    expect(store.getState().commandBarOpen).toBe(false);
  });
});
```

(`createGraphStore({ commandBarOpen: true })` works because overrides spread into initial state — Task 9 added the field.)

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @pensieve-ai/react test -- CommandBar`
Expected: FAIL.

- [ ] **Step 3: Implement**

```tsx
/**
 * CommandBar — ⌘K glass panel, top-center. One box for: node search with
 * fly-to (Enter/click), arrow-key navigation, and Esc to close. Searches the
 * already-loaded graphology instance (no server round-trip — the full graph
 * is local).
 */
import { useEffect, useMemo, useRef, useState } from "react";
import type Graph from "graphology";
import { Search } from "lucide-react";
import { getLabelColor } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";

const MAX_RESULTS = 20;

export function CommandBar({ graphRef }: { graphRef: React.RefObject<Graph | null> }) {
  const open = useGraphStore((s) => s.commandBarOpen);
  const setOpen = useGraphStore((s) => s.setCommandBarOpen);
  const focusNode = useGraphStore((s) => s.focusNode);
  const pushTrail = useGraphStore((s) => s.pushTrail);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Global ⌘K / Ctrl+K.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen(!open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, setOpen]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const results = useMemo(() => {
    const graph = graphRef.current;
    const q = query.trim().toLowerCase();
    if (!graph || !q) return [];
    const out: Array<{ id: string; label: string; nodeLabel: string }> = [];
    graph.forEachNode((id, attrs) => {
      if (out.length >= MAX_RESULTS) return;
      const label = String(attrs.label ?? "");
      if (label.toLowerCase().includes(q) || id.toLowerCase().includes(q)) {
        out.push({ id, label, nodeLabel: String(attrs.nodeLabel ?? "Node") });
      }
    });
    return out;
  }, [graphRef, query]);

  if (!open) return null;

  const select = (id: string) => {
    pushTrail(id);
    focusNode(id); // fly-to via focusSeq
    setOpen(false);
  };

  return (
    <div className="pv-absolute pv-left-1/2 pv-top-6 pv-z-30 pv-w-[480px] pv-max-w-[90%] -pv-translate-x-1/2">
      <div className="pv-glass pv-rounded-xl pv-border pv-border-border pv-shadow-elev-3">
        <div className="pv-flex pv-items-center pv-gap-2 pv-border-b pv-border-border pv-px-3 pv-py-2.5">
          <Search className="pv-h-4 pv-w-4 pv-text-muted-foreground" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") setOpen(false);
              else if (e.key === "ArrowDown") setActive((a) => Math.min(a + 1, results.length - 1));
              else if (e.key === "ArrowUp") setActive((a) => Math.max(a - 1, 0));
              else if (e.key === "Enter" && results[active]) select(results[active].id);
            }}
            placeholder="Search nodes, fly to anything…"
            className="pv-w-full pv-bg-transparent pv-text-sm pv-text-foreground pv-outline-none placeholder:pv-text-muted-foreground"
          />
          <kbd className="pv-rounded pv-border pv-border-border pv-px-1.5 pv-py-0.5 pv-text-2xs pv-text-muted-foreground">esc</kbd>
        </div>
        {results.length > 0 && (
          <ul className="pv-max-h-80 pv-overflow-y-auto pv-py-1">
            {results.map((r, i) => (
              <li key={r.id}>
                <button
                  type="button"
                  onClick={() => select(r.id)}
                  onMouseEnter={() => setActive(i)}
                  className={`pv-flex pv-w-full pv-items-center pv-gap-2 pv-px-3 pv-py-1.5 pv-text-left pv-text-sm ${
                    i === active ? "pv-bg-accent pv-text-foreground" : "pv-text-muted-foreground"
                  }`}
                >
                  <span
                    className="pv-h-2 pv-w-2 pv-shrink-0 pv-rounded-full"
                    style={{ background: getLabelColor(r.nodeLabel) }}
                  />
                  <span className="pv-truncate">{r.label}</span>
                  <span className="pv-ml-auto pv-shrink-0 pv-text-2xs pv-text-muted-foreground">{r.nodeLabel}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {query.trim() && results.length === 0 && (
          <div className="pv-px-3 pv-py-3 pv-text-xs pv-text-muted-foreground">No matching nodes.</div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run tests**

Run: `pnpm --filter @pensieve-ai/react test -- CommandBar`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/react/src/graph/CommandBar.tsx packages/react/src/graph/CommandBar.test.tsx
git commit -m "feat(react): cmd-k command bar with local node search and fly-to"
```

---

### Task 12: inspector panel, legend dock, breadcrumbs, minimap

Four presentational components. They read store + props; logic-heavy parts (display, walk, trail) are already tested, so these get render smoke tests only.

**Files:**
- Create: `packages/react/src/graph/InspectorPanel.tsx`
- Create: `packages/react/src/graph/LegendDock.tsx`
- Create: `packages/react/src/graph/BreadcrumbTrail.tsx`
- Create: `packages/react/src/graph/Minimap.tsx`
- Test: `packages/react/src/graph/chrome.test.tsx` (one smoke test per component)

- [ ] **Step 1: InspectorPanel** — slides in from the right when a node is selected. Contents and behaviors (port the inspector section markup style from `GraphSidebar.tsx` — node header, properties table, metadata):

```tsx
/**
 * InspectorPanel — floating glass panel, right side, mounted only while a
 * node is selected. Properties, metadata, and incident edges grouped by
 * relationship type with direction glyphs; per-edge fly-to; focus-
 * neighborhood + copy-id actions.
 */
import { useMemo } from "react";
import { ArrowLeft, ArrowRight, Copy, Crosshair, X } from "lucide-react";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";
import { getRelationshipFamilyColor } from "./graph-style";

const keyOf = (n: { id: string; namespace?: string }) => `${n.namespace ?? ""}::${n.id}`;

export function InspectorPanel({
  node,
  edges,
  nodesByCompositeId,
}: {
  node: GraphNode | null;
  edges: GraphRelationship[];
  nodesByCompositeId: Map<string, GraphNode>;
}) {
  const selectNode = useGraphStore((s) => s.selectNode);
  const focusNode = useGraphStore((s) => s.focusNode);
  const pushTrail = useGraphStore((s) => s.pushTrail);
  const focusModeId = useGraphStore((s) => s.focusModeId);
  const setFocusMode = useGraphStore((s) => s.setFocusMode);

  const incident = useMemo(() => {
    if (!node) return new Map<string, Array<{ edge: GraphRelationship; otherKey: string; out: boolean }>>();
    const key = keyOf(node);
    const groups = new Map<string, Array<{ edge: GraphRelationship; otherKey: string; out: boolean }>>();
    for (const e of edges) {
      const src = `${e.namespace ?? ""}::${e.source_id}`;
      const dst = `${(e.properties?.target_namespace as string | undefined) ?? e.namespace ?? ""}::${e.target_id}`;
      if (src !== key && dst !== key) continue;
      const out = src === key;
      const arr = groups.get(e.relationship_type) ?? [];
      arr.push({ edge: e, otherKey: out ? dst : src, out });
      groups.set(e.relationship_type, arr);
    }
    return groups;
  }, [node, edges]);

  if (!node) return null;
  const nodeKey = keyOf(node);
  const name = (node.properties?.name as string) || (node.properties?.title as string) || node.id;

  return (
    <div className="pv-absolute pv-right-4 pv-top-16 pv-bottom-20 pv-z-20 pv-w-80 pv-overflow-y-auto pv-rounded-xl pv-border pv-border-border pv-glass pv-shadow-elev-3 pv-animate-fade-in">
      <div className="pv-flex pv-items-start pv-justify-between pv-border-b pv-border-border pv-p-3">
        <div className="pv-min-w-0">
          <div className="pv-truncate pv-text-sm pv-font-medium pv-text-foreground">{name}</div>
          <div className="pv-text-2xs pv-text-muted-foreground">
            {node.labels.join(" · ")} · {node.namespace}
          </div>
        </div>
        <button type="button" onClick={() => selectNode(null)} className="pv-text-muted-foreground hover:pv-text-foreground">
          <X className="pv-h-4 pv-w-4" />
        </button>
      </div>

      <div className="pv-flex pv-gap-2 pv-p-3">
        <button
          type="button"
          onClick={() => setFocusMode(focusModeId === nodeKey ? null : nodeKey)}
          className="pv-flex pv-items-center pv-gap-1 pv-rounded-md pv-border pv-border-border pv-px-2 pv-py-1 pv-text-xs pv-text-muted-foreground hover:pv-text-foreground"
        >
          <Crosshair className="pv-h-3 pv-w-3" />
          {focusModeId === nodeKey ? "Exit focus" : "Focus neighborhood"}
        </button>
        <button
          type="button"
          onClick={() => void navigator.clipboard.writeText(node.id)}
          className="pv-flex pv-items-center pv-gap-1 pv-rounded-md pv-border pv-border-border pv-px-2 pv-py-1 pv-text-xs pv-text-muted-foreground hover:pv-text-foreground"
        >
          <Copy className="pv-h-3 pv-w-3" /> Copy id
        </button>
      </div>

      {Object.keys(node.properties ?? {}).length > 0 && (
        <div className="pv-border-t pv-border-border pv-p-3">
          <div className="pv-mb-1 pv-text-2xs pv-uppercase pv-text-muted-foreground">Properties</div>
          <dl className="pv-space-y-1">
            {Object.entries(node.properties).slice(0, 30).map(([k, v]) => (
              <div key={k} className="pv-flex pv-gap-2 pv-text-xs">
                <dt className="pv-w-28 pv-shrink-0 pv-truncate pv-text-muted-foreground">{k}</dt>
                <dd className="pv-min-w-0 pv-truncate pv-font-mono pv-text-foreground">{String(v)}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}

      <div className="pv-border-t pv-border-border pv-p-3">
        <div className="pv-mb-1 pv-text-2xs pv-uppercase pv-text-muted-foreground">Relationships</div>
        {[...incident.entries()].map(([relType, items]) => (
          <div key={relType} className="pv-mb-2">
            <div className="pv-flex pv-items-center pv-gap-1.5 pv-text-2xs pv-font-mono" style={{ color: getRelationshipFamilyColor(relType) }}>
              {relType} <span className="pv-text-muted-foreground">({items.length})</span>
            </div>
            <ul className="pv-mt-0.5 pv-space-y-0.5">
              {items.slice(0, 25).map(({ edge, otherKey, out }) => {
                const other = nodesByCompositeId.get(otherKey);
                const otherName = other
                  ? (other.properties?.name as string) || other.id
                  : otherKey.split("::").pop();
                return (
                  <li key={edge.id}>
                    <button
                      type="button"
                      onClick={() => {
                        pushTrail(otherKey);
                        focusNode(otherKey);
                      }}
                      className="pv-flex pv-w-full pv-items-center pv-gap-1.5 pv-rounded pv-px-1 pv-py-0.5 pv-text-left pv-text-xs pv-text-muted-foreground hover:pv-bg-accent hover:pv-text-foreground"
                    >
                      {out ? <ArrowRight className="pv-h-3 pv-w-3 pv-shrink-0" /> : <ArrowLeft className="pv-h-3 pv-w-3 pv-shrink-0" />}
                      <span className="pv-truncate">{otherName}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
        {incident.size === 0 && <div className="pv-text-xs pv-text-muted-foreground">No edges.</div>}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: LegendDock** — bottom-center chip → expandable panel. Port the filter/toggle sections from `GraphSidebar.tsx` (node-type visibility with counts, namespace visibility, relationship-type isolation via `setRelTypeFilter`, the 6 style toggles, the 4-button layout picker wired to `setLayout`). Collapsed chip: relationship-family color swatches + `{visibleNodes} nodes · {visibleEdges} edges`. Expanded: `pv-glass` panel (max-h-96, overflow-y-auto) with those sections. Props:

```tsx
export function LegendDock(props: {
  stats: GraphStats | null;
  coords: GraphCoord[];
  namespaceCounts: Record<string, number>;
  visibleNodes: number;
  visibleEdges: number;
}): JSX.Element
```

All state flows through the existing store actions (`toggleHiddenLabel`, `toggleHiddenNamespace`, `setRelTypeFilter`, `toggleGlow`, `toggleAnimatedFlow`, `toggleCurvedEdges`, `toggleCommunityHulls`, `toggleSizeByDegree`, `toggleEdgeLabels`, `setLayout`). Note `setLayout` now triggers an export refetch — wired in Task 13.

- [ ] **Step 3: BreadcrumbTrail** — top-left chips:

```tsx
/** BreadcrumbTrail — visited nodes, click to fly back (jumpTrail). */
import { ChevronRight } from "lucide-react";
import type { GraphNode } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";

export function BreadcrumbTrail({ nodesByCompositeId }: { nodesByCompositeId: Map<string, GraphNode> }) {
  const trail = useGraphStore((s) => s.trail);
  const jumpTrail = useGraphStore((s) => s.jumpTrail);
  if (trail.length === 0) return null;
  const visible = trail.slice(-5);
  const offset = trail.length - visible.length;
  return (
    <div className="pv-absolute pv-left-4 pv-top-6 pv-z-20 pv-flex pv-max-w-[60%] pv-items-center pv-gap-0.5 pv-rounded-full pv-glass pv-border pv-border-border pv-px-2 pv-py-1">
      {offset > 0 && <span className="pv-text-2xs pv-text-muted-foreground">…</span>}
      {visible.map((id, i) => {
        const node = nodesByCompositeId.get(id);
        const name = node ? (node.properties?.name as string) || node.id : id.split("::").pop();
        const last = i === visible.length - 1;
        return (
          <span key={`${id}-${i}`} className="pv-flex pv-items-center pv-gap-0.5">
            {(i > 0 || offset > 0) && <ChevronRight className="pv-h-3 pv-w-3 pv-text-muted-foreground" />}
            <button
              type="button"
              onClick={() => jumpTrail(offset + i)}
              className={`pv-max-w-32 pv-truncate pv-text-xs ${last ? "pv-text-foreground" : "pv-text-muted-foreground hover:pv-text-foreground"}`}
            >
              {name}
            </button>
          </span>
        );
      })}
    </div>
  );
}
```

- [ ] **Step 4: Minimap** — bottom-left density map + draggable viewport:

```tsx
/**
 * Minimap — downsampled whole-graph dot map with the camera viewport
 * rectangle. Click or drag moves the camera. Redraws on sigma afterRender
 * (throttled by rAF coalescing in sigma itself).
 */
import { useEffect, useRef } from "react";
import type Sigma from "sigma";

const W = 180;
const H = 120;

export function Minimap({ sigma }: { sigma: Sigma | null }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !sigma) return;
    const graph = sigma.getGraph();

    // Graph-space extent (positions are static — compute once per graph).
    let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
    graph.forEachNode((_, a) => {
      minX = Math.min(minX, a.x as number); maxX = Math.max(maxX, a.x as number);
      minY = Math.min(minY, a.y as number); maxY = Math.max(maxY, a.y as number);
    });
    const spanX = Math.max(maxX - minX, 1e-6);
    const spanY = Math.max(maxY - minY, 1e-6);
    const toMini = (x: number, y: number) => ({
      x: ((x - minX) / spanX) * (W - 8) + 4,
      y: ((y - minY) / spanY) * (H - 8) + 4,
    });

    const draw = () => {
      const ctx = canvas.getContext("2d")!;
      const dpr = window.devicePixelRatio;
      canvas.width = W * dpr;
      canvas.height = H * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);
      ctx.fillStyle = "rgba(148,163,184,0.55)";
      const step = Math.max(1, Math.floor(graph.order / 1500)); // ≤1500 dots
      let i = 0;
      graph.forEachNode((_, a) => {
        if (i++ % step !== 0) return;
        const p = toMini(a.x as number, a.y as number);
        ctx.fillRect(p.x, p.y, 1.5, 1.5);
      });
      // Viewport rect: corners of the screen mapped into graph space.
      const tl = sigma.viewportToGraph({ x: 0, y: 0 });
      const br = sigma.viewportToGraph({ x: sigma.getDimensions().width, y: sigma.getDimensions().height });
      const a = toMini(tl.x, tl.y);
      const b = toMini(br.x, br.y);
      ctx.strokeStyle = "#22d3ee";
      ctx.lineWidth = 1.2;
      ctx.strokeRect(a.x, a.y, b.x - a.x, b.y - a.y);
    };

    const moveCamera = (clientX: number, clientY: number) => {
      const rect = canvas.getBoundingClientRect();
      const gx = ((clientX - rect.left - 4) / (W - 8)) * spanX + minX;
      const gy = ((clientY - rect.top - 4) / (H - 8)) * spanY + minY;
      const framed = sigma.getCamera().getState();
      // graphToFramedGraph normalization: use sigma's helper.
      const f = sigma.getCustomBBox() ?? sigma.getBBox();
      void f; // bbox already reflected in normalization below
      const norm = sigma.graphToViewport({ x: gx, y: gy });
      const center = sigma.viewportToFramedGraph(norm);
      sigma.getCamera().animate({ ...framed, x: center.x, y: center.y }, { duration: 150 });
    };

    let dragging = false;
    const down = (e: MouseEvent) => { dragging = true; moveCamera(e.clientX, e.clientY); };
    const move = (e: MouseEvent) => { if (dragging) moveCamera(e.clientX, e.clientY); };
    const up = () => { dragging = false; };
    canvas.addEventListener("mousedown", down);
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    draw();
    sigma.on("afterRender", draw);
    return () => {
      sigma.off("afterRender", draw);
      canvas.removeEventListener("mousedown", down);
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
  }, [sigma]);

  return (
    <div className="pv-absolute pv-bottom-4 pv-left-4 pv-z-20 pv-rounded-lg pv-glass pv-border pv-border-border pv-p-1">
      <canvas ref={canvasRef} style={{ width: W, height: H }} className="pv-cursor-crosshair pv-rounded" />
    </div>
  );
}
```

The camera-centering math in `moveCamera` is the fiddly part — verify against the installed sigma version's `viewportToFramedGraph`/`graphToViewport` helpers and adjust until clicking the minimap centers that spot.

- [ ] **Step 5: Smoke tests**

`chrome.test.tsx`: render each component inside `GraphStoreContext.Provider` (store from `createGraphStore()`); assert InspectorPanel shows node name + relationship group, BreadcrumbTrail renders trail entries and `jumpTrail` fires on click, LegendDock chip shows counts and expands on click, Minimap renders a canvas with `sigma={null}` without crashing.

Run: `pnpm --filter @pensieve-ai/react test -- chrome`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/react/src/graph/InspectorPanel.tsx packages/react/src/graph/LegendDock.tsx packages/react/src/graph/BreadcrumbTrail.tsx packages/react/src/graph/Minimap.tsx packages/react/src/graph/chrome.test.tsx
git commit -m "feat(react): graph chrome — inspector, legend dock, breadcrumbs, minimap"
```

---

### Task 13: full-bleed GraphView assembly + fallback + public API

**Files:**
- Rewrite: `packages/react/src/graph/GraphView.tsx`
- Modify: `packages/react/src/graph/PensieveGraph.tsx` (new props `renderer`, `chrome`)
- Modify: `packages/react/src/graph/GraphSidebar.tsx` → delete, plus `GraphSidebar.test.tsx`
- Modify: `packages/react/src/graph/index.ts` (exports — check current contents first)
- Test: existing `PensieveGraph.test.tsx` (update), `pnpm --filter @pensieve-ai/react test`

- [ ] **Step 1: Rewrite GraphView**

Keep: store wiring, namespace/label filtering, deep-link focus, galaxy substrate + vignette, empty/loading/error states, `onSelectedNodeChange` bridge. Remove: `OVERVIEW_LIMIT`/`LANDMARK_COUNT` overview reduction (lines 21-24, 154-180 of the old file), `GraphSidebar` usage. Add: `useGraphExport` data, SigmaCanvas + chrome assembly, WebGL fallback, layout-computing progress, partial-load retry banner.

Core structure (complete — filtering/dedup helpers carry over from the old file):

```tsx
export function webglAvailable(): boolean {
  try {
    const c = document.createElement("canvas");
    return Boolean(c.getContext("webgl2") ?? c.getContext("webgl"));
  } catch {
    return false;
  }
}

export interface GraphViewProps {
  graphs?: Array<{ database?: string; graph: string }>;
  discover?: "all-databases";
  realm?: string;
  showChrome?: boolean;            // replaces showSidebar
  renderer?: "webgl" | "canvas";   // default: auto-detect
  onSelectedNodeChange?: (node: GraphNode | null) => void;
  focusNodeId?: string;
}

export function GraphView({ graphs, discover, realm, showChrome = true, renderer, onSelectedNodeChange, focusNodeId }: GraphViewProps) {
  const layout = useGraphStore((s) => s.layout);
  // … existing store selectors …
  const useWebgl = renderer ? renderer === "webgl" : webglAvailable();

  const exp = useGraphExport({ graphs, discover, realm, algorithm: layout });
  const { nodes, edges, positions } = exp.acc;
  // … activeNamespaces / nodesByCompositeId / deep-link focus — same as old file,
  //    but over exp.acc.nodes (note: namespace/label filtering for the sigma path
  //    happens inside buildGraphologyGraph; nodesByCompositeId stays unfiltered) …

  const sigmaRef = useRef<Sigma | null>(null);
  const graphRef = useRef<Graph | null>(null); // set from sigma.getGraph() in onSigmaReady
  useKeyboardWalk(graphRef);

  const selectAndTrail = (id: string) => {
    if (id === "") { selectNode(null); return; }
    pushTrail(id);
    selectNode(id);
  };

  return (
    <div className="pv-relative pv-h-full pv-w-full pv-overflow-hidden pv-bg-background">
      <div className="pv-pointer-events-none pv-absolute pv-inset-0" style={{ background: galaxyBg }} />
      {/* hulls under the sigma canvases */}
      {useWebgl && <HullsLayer sigma={sigmaRef.current} nodes={nodes} edges={edges} version={exp.version} />}
      {useWebgl ? (
        <SigmaCanvas
          nodes={nodes} edges={edges} positions={positions} version={exp.version}
          activeNamespaces={activeNamespaces}
          onNodeClick={selectAndTrail}
          onNodeHover={hoverNode}
          onNodeDoubleClick={(id) => setFocusMode(focusModeId === id ? null : id)}
          onSigmaReady={(s) => { sigmaRef.current = s; graphRef.current = s.getGraph(); }}
        />
      ) : (
        <GraphCanvas /* legacy fallback — same props as before, fed from exp.acc */
          nodes={visNodes} edges={visEdges} layout={layout} showEdgeLabels={showEdgeLabels}
          onNodeClick={selectAndTrail} onNodeHover={hoverNode}
          onNodeDoubleClick={(id) => setFocusMode(focusModeId === id ? null : id)}
        />
      )}
      {/* vignette (unchanged) */}
      {exp.layoutComputing && (
        <div className="pv-absolute pv-left-1/2 pv-top-16 pv-z-10 -pv-translate-x-1/2 pv-glass pv-rounded-full pv-px-3 pv-py-1 pv-text-xs pv-text-muted-foreground">
          Computing layout server-side… {exp.acc.nodes.length.toLocaleString()} nodes loaded
        </div>
      )}
      {exp.isError && exp.acc.nodes.length > 0 && (
        <div className="pv-absolute pv-bottom-20 pv-left-1/2 pv-z-10 -pv-translate-x-1/2 pv-glass pv-rounded-full pv-px-3 pv-py-1 pv-text-xs pv-text-destructive">
          Partial graph — a stream failed. <button type="button" onClick={exp.refetch} className="pv-underline">Retry</button>
        </div>
      )}
      {showChrome && (
        <>
          <CommandBar graphRef={graphRef} />
          <BreadcrumbTrail nodesByCompositeId={nodesByCompositeId} />
          <InspectorPanel node={selectedNode} edges={edges} nodesByCompositeId={nodesByCompositeId} />
          <LegendDock stats={exp.acc.stats} coords={exp.coords} namespaceCounts={namespaceCounts} visibleNodes={nodes.length} visibleEdges={edges.length} />
          <Minimap sigma={sigmaRef.current} />
          {/* zoom controls: bottom-right +/− /fit buttons calling sigma camera animatedZoom/animatedReset */}
        </>
      )}
      {/* loading / error / empty states — same as old file, driven by exp */}
    </div>
  );
}
```

`namespaceCounts` derives from `exp.acc.nodes` (count per `namespace`). The layout picker in LegendDock calls `setLayout` → `layout` changes → `useGraphExport` algorithm arg changes → automatic refetch with the new server-side layout.

- [ ] **Step 2: PensieveGraph props**

In `PensieveGraph.tsx`: add `renderer?: "webgl" | "canvas"` and `chrome?: boolean` (default true); keep `toolbar`/`sidebar` working — `showChrome = (chrome ?? true) && sidebar && toolbar`. Pass `renderer` + `showChrome` to GraphView. Update the doc comment (the sidebar note is obsolete). `limit` prop: keep accepted but unused (full graph always) — note it as deprecated in the JSDoc.

- [ ] **Step 3: Delete GraphSidebar + fix exports/tests**

```bash
git rm packages/react/src/graph/GraphSidebar.tsx packages/react/src/graph/GraphSidebar.test.tsx
```
Check `packages/react/src/graph/index.ts` for a GraphSidebar export and remove it. Update `PensieveGraph.test.tsx` expectations that referenced sidebar DOM.

- [ ] **Step 4: Full verification**

Run: `pnpm --filter @pensieve-ai/react test && pnpm --filter @pensieve-ai/react typecheck && pnpm --filter @pensieve-ai/react build`
Expected: all PASS, build clean. The web route `web/src/routes/_app.graph.tsx` needs no change (PensieveGraph defaults now render full-bleed chrome).

- [ ] **Step 5: Commit**

```bash
git add -A packages/react/src
git commit -m "feat(react): full-bleed graph explorer — sigma renderer, chrome assembly, canvas fallback"
```

---

## Phase E — Verification

### Task 14: benchmark fixture + end-to-end verification

**Files:**
- Create: `web/src/routes/_app.graph-bench.tsx`
- Modify: none else

- [ ] **Step 1: Synthetic benchmark route**

A dev-only route that generates a deterministic synthetic graph (no server) and feeds SigmaCanvas directly, with an FPS meter:

```tsx
/** /graph-bench?n=50000&e=100000 — synthetic perf fixture (dev builds only). */
import { createFileRoute } from "@tanstack/react-router";
// Generate n nodes in label groups with mulberry32-seeded positions spread in a
// 4000×2500 box and e random edges across 7 relationship types; build the same
// props SigmaCanvas takes (nodes, edges, positions Map, activeNamespaces).
// Overlay: requestAnimationFrame counter → fps readout in a corner chip.
```

(Match the file-route conventions of `web/src/routes/_app.graph.tsx`. Wrap in PensieveGraph's store context via `createGraphStore` + `GraphStoreContext.Provider`. Guard with `import.meta.env.DEV ? route : null` if the router requires static routes, otherwise just leave it — it's behind auth like other app routes.)

- [ ] **Step 2: Benchmark**

Run the web dev server (per `web/package.json` scripts), open `/graph-bench?n=50000&e=100000`.
Expected vs the spec budget:
- pan/zoom ≥ 55fps sustained
- hover highlight under one frame
- initial render < 2s after data generation

If fps falls short, the knobs are: `labelRenderedSizeThreshold` (raise), hide edges entirely at `far` tier when `graph.size > 150k` (add to `edgeDisplay`), and `hideEdgesOnMove: true` sigma setting. Apply and re-measure before touching anything else.

- [ ] **Step 3: Real-deployment verification checklist**

Against a seeded dev deployment (server from this branch):

1. `/graph` opens full-bleed; full graph loads progressively (watch node count grow in LegendDock chip); no overview-mode banner exists anymore.
2. Edges show quiet arrowheads at rest; hover a node → incident edges go loud (bold + labels + dimmed rest).
3. Click node → inspector slides in; relationship groups show direction glyphs; "fly to other end" works.
4. ⌘K → type → Enter → camera flies to the node; breadcrumb appears; Esc steps back.
5. Tab cycles neighbors with highlight; Enter walks the edge.
6. Double-click isolates the neighborhood; double-click again (or Exit focus) restores.
7. Minimap rect tracks pan/zoom; dragging it moves the camera.
8. Layout picker: switching force → tree refetches export and re-renders with new positions.
9. `?focus=<nodeId>` deep-link still centers + zooms.
10. Large graph (>20k nodes): first response shows "Computing layout server-side…", then fills in; second visit is instant (cache).
11. Force `renderer="canvas"`: legacy renderer still works with the new chrome.

- [ ] **Step 4: Commit**

```bash
git add web/src/routes/_app.graph-bench.tsx
git commit -m "feat(web): synthetic graph benchmark route for renderer perf verification"
```

---

## Self-review notes (already applied)

- **Spec coverage:** §1 renderer → Tasks 6-8; §2 export/layout/cache + cap removal → Tasks 1-5, 13; §3 chrome → Tasks 11-13; §4 navigation → Tasks 9-13; §5 store/API compat → Tasks 9, 13; §6 error handling → Tasks 3 (GONE/400), 5 (poll/partial), 13 (banner, fallback); §7-8 budgets/testing → Task 14 + per-task tests.
- **Deliberate deviations from spec:** none. `limit` prop retained-but-deprecated per spec §5.
- **Type consistency spots checked:** `LayoutAlgorithm` (client string union ↔ Rust enum, serde lowercase matches); composite-id scheme `${namespace}::${id}` used identically in merge/build/walk/inspector; cursor grammar `layout_id:kind:offset` in Task 2 ↔ Task 3 ↔ client passthrough.
- **Known verify-on-site points (flagged inline):** sigma v3 minor API drift (reducers/camera helpers), `radiusForDegree` signature, `detectCommunities` return shape, client transport request helper name, `QueryState` construction sites.

---

## Amendment — reference-image visual direction (2026-06-11)

Overrides specifics in Tasks 6–7 and adds Task 1b. See the spec's same-dated amendment.

- **Task 1b (new, server):** in `crates/pensieve-graph/src/layout.rs`, add a deterministic orbit post-pass to `force_layout`: collect degree-1 nodes per hub (their sole neighbor); for each hub with ≥4 leaves, place its leaves on concentric rings centered on the hub — ring radius `r_k = 90 + 55*k`, ring capacity `floor(2π r_k / 34)`, angles evenly spaced starting at the hub's angle-to-graph-center (deterministic), leaves ordered by id. Then re-run the existing overlap pass. Tests: star(60) → all leaves within ring radii of hub ± tolerance and min pairwise leaf distance ≥ 25; determinism re-asserted.
- **Task 6 override (`edgeDisplay`):** at-rest edges return a NEUTRAL color signal (`loud: false` → renderer uses neutral hairline rgba(148,163,184,alpha) with alpha 0.16 far / 0.22 mid-near dark mode, 0.28 light), ignoring family color. Loud state (incident to focus / relType isolation match) uses the family color as before. `edgeDisplay` gains a `neutral: boolean` field (true at rest, false when loud) so the reducer picks the palette.
- **Task 7 override (`buildGraphologyGraph` + reducers):** node `image` attribute still computed, but the node reducer only renders type "image" at mid/near LOD or for landmark hubs (size ≥ landmark threshold — top-decile degree); otherwise plain circle. Radius range widens: `radiusForDegree` output rescaled to [2.5, 22] so hubs visibly dominate. Edge reducer uses `neutral` per Task 6 override.
