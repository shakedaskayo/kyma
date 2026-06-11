/**
 * HullsLayer — translucent community blobs under the sigma canvases.
 * Communities come from detectCommunities (label propagation); hull points
 * are projected with sigma.graphToViewport on every render so they track
 * camera moves exactly.
 *
 * detectCommunities returns Map<nodeId, communityIndex> (not Map<communityId,
 * members[]>). groupByCommunity inverts that into Map<communityIndex, nodeId[]>
 * so the draw loop can iterate communities and get their member node ids.
 */
import { useEffect, useMemo, useRef } from "react";
import type Sigma from "sigma";
import type { GraphNode, GraphRelationship } from "@kyma-ai/client";
import { getLabelColor } from "@kyma-ai/client";
import { convexHull, detectCommunities, padHull } from "./graph-community";
import { edgeDstKey, edgeSrcKey, keyOf } from "./sigma-graph-builder";
import { useGraphStore } from "./graph-store";

// ── Grouping helper (exported for tests) ──────────────────────────────────

/**
 * Invert a nodeId→communityIndex map into communityIndex→nodeIds[].
 * Pure function — no side effects, deterministic, unit-testable.
 */
export function groupByCommunity(
  assignment: Map<string, number>,
): Map<number, string[]> {
  const out = new Map<number, string[]>();
  for (const [nodeId, communityIdx] of assignment) {
    let members = out.get(communityIdx);
    if (!members) {
      members = [];
      out.set(communityIdx, members);
    }
    members.push(nodeId);
  }
  return out;
}

// ── Component ──────────────────────────────────────────────────────────────

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

  // Detect communities memoized on version + toggle.
  // detectCommunities takes string[] nodeIds and {source, target}[] edges —
  // map from the GraphNode/GraphRelationship shapes before calling.
  const communities = useMemo((): Map<number, string[]> | null => {
    if (!communityHulls) return null;
    // Composite ids — the sigma graph keys nodes by `${namespace}::${id}`,
    // so community membership must use the same scheme or hasNode() below
    // never matches and no hull ever draws.
    const nodeIds = nodes.map(keyOf);
    const edgePairs = edges.map((e) => ({
      source: edgeSrcKey(e),
      target: edgeDstKey(e),
    }));
    const assignment = detectCommunities(nodeIds, edgePairs);
    return groupByCommunity(assignment);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [version, communityHulls]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !sigma) return;

    const draw = () => {
      const { width, height } = sigma.getDimensions();
      const dpr = window.devicePixelRatio;
      canvas.width = width * dpr;
      canvas.height = height * dpr;
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      const ctx = canvas.getContext("2d")!;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, width, height);
      if (!communities) return;

      const graph = sigma.getGraph();
      for (const [communityIdx, memberIds] of communities) {
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
        // Use a deterministic color keyed on the community index string.
        const color = getLabelColor(String(communityIdx));
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
  return (
    <canvas
      ref={canvasRef}
      className="ky-pointer-events-none ky-absolute ky-inset-0"
    />
  );
}
