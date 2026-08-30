/**
 * Pure graph-building logic for SigmaCanvas — no Sigma/WebGL imports so this
 * module is safe to test in jsdom.
 *
 * Amendment (2026-06-11):
 * - Node radius rescaled from radiusForDegree's [5.5, 14] into [2.5, 22].
 * - Hub/leaf distinction: landmark = top-decile degree.
 */
import Graph from "graphology";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { getRelationshipFamilyColor, radiusForDegree } from "./graph-style";
import { getIconDataUrl, resolveGraphIcon, resolveNodeColor } from "./graph-icons";

// ── Helpers ────────────────────────────────────────────────────────────────

export const keyOf = (n: { id: string; namespace?: string }): string =>
  `${n.namespace ?? ""}::${n.id}`;

export const edgeSrcKey = (e: GraphRelationship): string =>
  `${(e as unknown as { namespace?: string }).namespace ?? ""}::${e.source_id}`;

export const edgeDstKey = (e: GraphRelationship): string =>
  `${
    ((e.properties?.target_namespace as string | undefined) ??
      (e as unknown as { namespace?: string }).namespace) ?? ""
  }::${e.target_id}`;

/**
 * Rescale a radius from radiusForDegree's output range [5.5, 14] linearly
 * into [2.5, 22] so hubs visually dominate over leaf nodes.
 *
 * radiusForDegree(degree, capDeg, sizeByDegree) returns base=5.5 (leaf) → 14 (hub).
 * Amendment target: [2.5, 22].
 */
const RADIUS_SRC_MIN = 5.5;
const RADIUS_SRC_MAX = 14;
export const RADIUS_DST_MIN = 2.5;
export const RADIUS_DST_MAX = 22;

/** Hard cap for canvas node labels — sentence-length names get an ellipsis. */
export const LABEL_MAX_CHARS = 56;

export function rescaleRadius(r: number): number {
  const t =
    (Math.max(RADIUS_SRC_MIN, Math.min(RADIUS_SRC_MAX, r)) - RADIUS_SRC_MIN) /
    (RADIUS_SRC_MAX - RADIUS_SRC_MIN);
  return RADIUS_DST_MIN + t * (RADIUS_DST_MAX - RADIUS_DST_MIN);
}

// ── Namespace tiling ────────────────────────────────────────────────────────

/**
 * Each graph is laid out server-side in its own independent coordinate space,
 * so a unified multi-graph view stacks every graph on top of the same region
 * (in the degenerate case, N single-node graphs all land on the exact same
 * point — zero bbox — which sigma's autoRescale blows up to fill the screen).
 *
 * Tile namespaces into a grid: each namespace keeps its internal layout but is
 * offset into its own cell. Deterministic (namespaces sorted) and a no-op for
 * single-namespace views. Exported for tests.
 */
export function namespaceTileOffsets(
  nodes: GraphNode[],
  positions: Map<string, { x: number; y: number }>,
): Map<string, { dx: number; dy: number }> {
  // Per-namespace bbox.
  const boxes = new Map<string, { minX: number; maxX: number; minY: number; maxY: number }>();
  for (const n of nodes) {
    const ns = (n as unknown as { namespace?: string }).namespace ?? "";
    const pos = positions.get(keyOf(n));
    if (!pos) continue;
    const b = boxes.get(ns);
    if (!b) {
      boxes.set(ns, { minX: pos.x, maxX: pos.x, minY: pos.y, maxY: pos.y });
    } else {
      b.minX = Math.min(b.minX, pos.x);
      b.maxX = Math.max(b.maxX, pos.x);
      b.minY = Math.min(b.minY, pos.y);
      b.maxY = Math.max(b.maxY, pos.y);
    }
  }
  const out = new Map<string, { dx: number; dy: number }>();
  if (boxes.size <= 1) return out; // single namespace keeps its native layout

  const MARGIN = 320;
  const cellW = Math.max(...[...boxes.values()].map((b) => b.maxX - b.minX), 400) + MARGIN;
  const cellH = Math.max(...[...boxes.values()].map((b) => b.maxY - b.minY), 400) + MARGIN;
  const namespaces = [...boxes.keys()].sort();
  const cols = Math.ceil(Math.sqrt(namespaces.length));
  namespaces.forEach((ns, i) => {
    const b = boxes.get(ns)!;
    const col = i % cols;
    const row = Math.floor(i / cols);
    // Center each namespace's bbox inside its cell.
    out.set(ns, {
      dx: col * cellW - b.minX + (cellW - (b.maxX - b.minX)) / 2,
      dy: row * cellH - b.minY + (cellH - (b.maxY - b.minY)) / 2,
    });
  });
  return out;
}

/**
 * Largest axis of the graph's position bounding box. Near-zero for a single
 * node or coincident nodes — sigma's autoRescale divides by this extent, so
 * graph-space node sizes explode to fill the screen. The renderer switches to
 * screen-based sizing below `4 × RADIUS_DST_MAX`. Exported for tests.
 */
export function positionExtent(g: Graph): number {
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  g.forEachNode((_, a) => {
    minX = Math.min(minX, a.x as number);
    maxX = Math.max(maxX, a.x as number);
    minY = Math.min(minY, a.y as number);
    maxY = Math.max(maxY, a.y as number);
  });
  if (!Number.isFinite(minX)) return 0;
  return Math.max(maxX - minX, maxY - minY);
}

// ── Public API ─────────────────────────────────────────────────────────────

export interface BuildOptions {
  sizeByDegree: boolean;
  hiddenLabels: string[];
  activeNamespaces: Set<string>;
}

/**
 * Build the graphology directed-multi-graph from positioned node/edge data.
 * Exported for unit tests (which cannot instantiate Sigma in jsdom).
 *
 * Node attrs: x, y, size (rescaled [2.5,22]), color, label, image (data-URL|undefined),
 *             isLandmark (top-decile degree), nodeLabel.
 * Edge attrs: size (1), relType, familyColor.
 */
export function buildGraphologyGraph(
  nodes: GraphNode[],
  edges: GraphRelationship[],
  positions: Map<string, { x: number; y: number }>,
  opts: BuildOptions,
): Graph {
  const g = new Graph({ multi: true, type: "directed" });

  // Compute per-composite-id degree from the raw edge list.
  const degree = new Map<string, number>();
  for (const e of edges) {
    const s = edgeSrcKey(e);
    const t = edgeDstKey(e);
    degree.set(s, (degree.get(s) ?? 0) + 1);
    degree.set(t, (degree.get(t) ?? 0) + 1);
  }

  // capDeg = max degree — the most-connected node defines the scale.
  // For very large real-world graphs (power-law distributions) this naturally
  // maps hubs to size 22 and leaves near 2.5.
  // landmarkThreshold: landmarks are the few hubs that keep their icon + label
  // even zoomed all the way out. Cap the count hard (top 1%, max 200) — a
  // percentile alone floods dense graphs with thousands of "landmarks" because
  // low-degree ties all clear the threshold together.
  const degs = [...degree.values()].sort((a, b) => a - b);
  const capDeg = degs.length ? degs[degs.length - 1] : 1;
  const landmarkCount = Math.min(60, Math.max(8, Math.floor(degs.length * 0.01)));
  const landmarkThreshold = degs.length
    ? Math.max(2, degs[Math.max(0, degs.length - landmarkCount)])
    : 1;

  // Offset each namespace's independent layout into its own grid cell so
  // unified views don't stack graphs on top of each other.
  const tiles = namespaceTileOffsets(nodes, positions);

  for (const n of nodes) {
    const ns = (n as unknown as { namespace?: string }).namespace ?? "";
    if (!opts.activeNamespaces.has(ns)) continue;
    if (opts.hiddenLabels.includes(n.labels[0] ?? "")) continue;

    const key = keyOf(n);
    const raw = positions.get(key) ?? { x: 0, y: 0 };
    const tile = tiles.get(ns);
    const pos = tile ? { x: raw.x + tile.dx, y: raw.y + tile.dy } : raw;
    const color = resolveNodeColor(n.labels, n.properties);
    const icon = resolveGraphIcon(n.labels, n.properties);
    // Canvas labels are wayfinding, not content: memory nodes carry whole
    // sentences as their name — truncate hard (full text lives in the
    // inspector and the hover pill gets the same truncated form).
    const rawLabel =
      (n.properties?.name as string | undefined) ||
      (n.properties?.title as string | undefined) ||
      n.id;
    const label = rawLabel.length > LABEL_MAX_CHARS
      ? `${rawLabel.slice(0, LABEL_MAX_CHARS - 1).trimEnd()}…`
      : rawLabel;
    const nodeDegree = degree.get(key) ?? 0;
    const rawRadius = radiusForDegree(nodeDegree, capDeg, opts.sizeByDegree);
    const size = rescaleRadius(rawRadius);
    const isLandmark = nodeDegree >= landmarkThreshold && landmarkThreshold > 0;

    g.addNode(key, {
      x: pos.x,
      y: pos.y,
      size,
      color,
      label,
      image: icon ? getIconDataUrl(icon, "#ffffff") : undefined,
      isLandmark,
      nodeLabel: n.labels[0] ?? "Node",
      // Default type — the reducer may override to "image" at mid/near LOD.
      type: "circle",
    });
  }

  for (const e of edges) {
    const s = edgeSrcKey(e);
    const t = edgeDstKey(e);
    if (!g.hasNode(s) || !g.hasNode(t)) continue;
    const ekey = `${(e as unknown as { namespace?: string }).namespace ?? ""}::${e.id}`;
    if (g.hasEdge(ekey)) continue;
    g.addDirectedEdgeWithKey(ekey, s, t, {
      size: 1,
      relType: e.relationship_type,
      familyColor: getRelationshipFamilyColor(e.relationship_type),
    });
  }

  return g;
}
