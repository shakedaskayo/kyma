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
