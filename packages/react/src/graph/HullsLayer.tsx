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
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { getLabelColor } from "@pensieve-ai/client";
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
      // Restraint: only sizeable communities, largest first, hard cap — hulls
      // are a whisper of grouping, not terrain. Tiny alphas keep them from
      // matting over the constellation on either theme.
      const sizeable = [...communities.entries()]
        .filter(([, m]) => m.length >= 8)
        .sort((a, b) => b[1].length - a[1].length)
        .slice(0, 12);
      for (const [communityIdx, memberIds] of sizeable) {
        const pts = memberIds
          .filter((id) => graph.hasNode(id))
          .map((id) => {
            const a = graph.getNodeAttributes(id);
            return sigma.graphToViewport({ x: a.x as number, y: a.y as number });
          });
        if (pts.length < 3) continue;
        const hull = padHull(convexHull(pts), 30);
        // Smooth, organic outline: quadratic curves through edge midpoints.
        ctx.beginPath();
        for (let i = 0; i < hull.length; i++) {
          const cur = hull[i];
          const next = hull[(i + 1) % hull.length];
          const mx = (cur.x + next.x) / 2;
          const my = (cur.y + next.y) / 2;
          if (i === 0) ctx.moveTo(mx, my);
          else ctx.quadraticCurveTo(cur.x, cur.y, mx, my);
        }
        // Close the loop around the FIRST vertex: the loop above ends at
        // midpoint(last, first), so the remaining arc bends through hull[0]
        // back to the starting midpoint(first, second). (Curving through
        // `last` again would add a degenerate zero-length arc + straight seam.)
        const first = hull[0];
        const second = hull[1];
        ctx.quadraticCurveTo(first.x, first.y, (first.x + second.x) / 2, (first.y + second.y) / 2);
        ctx.closePath();
        const color = getLabelColor(String(communityIdx));
        ctx.fillStyle = `${color}0A`;
        ctx.strokeStyle = `${color}1F`;
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
      className="pv-pointer-events-none pv-absolute pv-inset-0"
    />
  );
}
