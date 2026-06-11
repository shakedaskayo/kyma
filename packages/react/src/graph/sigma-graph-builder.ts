/**
 * Pure graph-building logic for SigmaCanvas — no Sigma/WebGL imports so this
 * module is safe to test in jsdom.
 *
 * Amendment (2026-06-11):
 * - Node radius rescaled from radiusForDegree's [5.5, 14] into [2.5, 22].
 * - Hub/leaf distinction: landmark = top-decile degree.
 */
import Graph from "graphology";
import type { GraphNode, GraphRelationship } from "@kyma-ai/client";
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

export function rescaleRadius(r: number): number {
  const t =
    (Math.max(RADIUS_SRC_MIN, Math.min(RADIUS_SRC_MAX, r)) - RADIUS_SRC_MIN) /
    (RADIUS_SRC_MAX - RADIUS_SRC_MIN);
  return RADIUS_DST_MIN + t * (RADIUS_DST_MAX - RADIUS_DST_MIN);
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
  // landmarkThreshold = 90th-percentile degree — top-decile nodes show icons at far LOD.
  const degs = [...degree.values()].sort((a, b) => a - b);
  const capDeg = degs.length ? degs[degs.length - 1] : 1;
  const landmarkThreshold = degs.length ? degs[Math.floor(degs.length * 0.9)] : 1;

  for (const n of nodes) {
    const ns = (n as unknown as { namespace?: string }).namespace ?? "";
    if (!opts.activeNamespaces.has(ns)) continue;
    if (opts.hiddenLabels.includes(n.labels[0] ?? "")) continue;

    const key = keyOf(n);
    const pos = positions.get(key) ?? { x: 0, y: 0 };
    const color = resolveNodeColor(n.labels, n.properties);
    const icon = resolveGraphIcon(n.labels, n.properties);
    const label =
      (n.properties?.name as string | undefined) ||
      (n.properties?.title as string | undefined) ||
      n.id;
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
