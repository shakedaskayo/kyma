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
