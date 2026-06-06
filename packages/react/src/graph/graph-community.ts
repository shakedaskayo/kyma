/**
 * Lightweight community detection (label propagation) + convex hull, used to
 * tint and wrap clusters in the graph "galaxy" view. Pure TS, deterministic,
 * O(iterations · edges) — no heavy graph-theory dependency.
 * Copied from web/src/features/graph/graph-community.ts.
 */

export interface Pt {
  x: number;
  y: number;
}

/**
 * Label propagation. Each node starts in its own community, then repeatedly
 * adopts the most common community among its neighbours. Deterministic: nodes
 * are processed in a fixed order and ties break toward the lower community id.
 * Returns a map id → community index, with indices sorted by community size
 * (0 = largest) so color assignment is stable across renders.
 */
export function detectCommunities(
  nodeIds: string[],
  edges: { source: string; target: string }[],
  iterations = 6,
): Map<string, number> {
  const out = new Map<string, number>();
  if (nodeIds.length === 0) return out;

  const adj = new Map<string, string[]>();
  for (const id of nodeIds) adj.set(id, []);
  for (const e of edges) {
    if (adj.has(e.source) && adj.has(e.target)) {
      adj.get(e.source)!.push(e.target);
      adj.get(e.target)!.push(e.source);
    }
  }

  // Initial label = own id.
  const label = new Map<string, string>();
  for (const id of nodeIds) label.set(id, id);

  const order = [...nodeIds].sort();
  for (let it = 0; it < iterations; it++) {
    let changed = false;
    for (const id of order) {
      const neighbors = adj.get(id)!;
      if (neighbors.length === 0) continue;
      const counts = new Map<string, number>();
      for (const nb of neighbors) {
        const l = label.get(nb)!;
        counts.set(l, (counts.get(l) ?? 0) + 1);
      }
      // Most frequent neighbour label; ties → lexicographically smallest.
      let best = label.get(id)!;
      let bestCount = -1;
      for (const [l, c] of counts) {
        if (c > bestCount || (c === bestCount && l < best)) {
          best = l;
          bestCount = c;
        }
      }
      if (best !== label.get(id)) {
        label.set(id, best);
        changed = true;
      }
    }
    if (!changed) break;
  }

  // Group → size, then assign indices by descending size for stable colors.
  const sizes = new Map<string, number>();
  for (const id of nodeIds) {
    const l = label.get(id)!;
    sizes.set(l, (sizes.get(l) ?? 0) + 1);
  }
  const ranked = [...sizes.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
  const indexOf = new Map<string, number>();
  ranked.forEach(([l], i) => indexOf.set(l, i));

  for (const id of nodeIds) out.set(id, indexOf.get(label.get(id)!)!);
  return out;
}

/** Andrew's monotone chain convex hull. Returns hull points CCW. */
export function convexHull(points: Pt[]): Pt[] {
  const pts = points.slice().sort((a, b) => a.x - b.x || a.y - b.y);
  if (pts.length <= 2) return pts;
  const cross = (o: Pt, a: Pt, b: Pt) =>
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);

  const lower: Pt[] = [];
  for (const p of pts) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0)
      lower.pop();
    lower.push(p);
  }
  const upper: Pt[] = [];
  for (let i = pts.length - 1; i >= 0; i--) {
    const p = pts[i];
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0)
      upper.pop();
    upper.push(p);
  }
  lower.pop();
  upper.pop();
  return lower.concat(upper);
}

/** Expand a hull outward from its centroid by `pad` graph units (for a soft blob). */
export function padHull(hull: Pt[], pad: number): Pt[] {
  if (hull.length === 0) return hull;
  const cx = hull.reduce((s, p) => s + p.x, 0) / hull.length;
  const cy = hull.reduce((s, p) => s + p.y, 0) / hull.length;
  return hull.map((p) => {
    const dx = p.x - cx;
    const dy = p.y - cy;
    const len = Math.hypot(dx, dy) || 1;
    return { x: p.x + (dx / len) * pad, y: p.y + (dy / len) * pad };
  });
}
