import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph2D, {
  type ForceGraphMethods,
  type NodeObject,
  type LinkObject,
} from "react-force-graph-2d";
import { Plus, Minus, Maximize2 } from "lucide-react";

import type { GraphNode, GraphRelationship } from "@/sdk/graph";
import { computeLayout, type LayoutAlgorithm } from "@/sdk/graph-layout";
import {
  getRelationshipFamilyColor,
  lighten,
  darken,
  alpha,
  radiusForDegree,
  LOD,
} from "./graph-style";
import { detectCommunities, convexHull, padHull } from "./graph-community";
import { resolveGraphIcon, resolveNodeColor, getIconImage, type ResolvedIcon } from "./graph-icons";
import { paletteFor } from "@/lib/data-palette";
import { useGraphStore } from "./graph-store";
import { useTheme } from "@/lib/theme";

export interface GraphCanvasProps {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  layout: LayoutAlgorithm;
  showEdgeLabels: boolean;
  onNodeClick: (id: string) => void;
  onNodeHover: (id: string | null) => void;
  onNodeDoubleClick?: (id: string) => void;
}

// Cap auto-fit zoom so a sparse graph (or a single node) never balloons to
// fill the viewport.
const MAX_FIT_ZOOM = 2.4;

// Trace a rounded-rect path (for label pills) — ctx.roundRect isn't in the TS
// lib target, so we build it from arcs.
function roundRectPath(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  const rad = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rad, y);
  ctx.arcTo(x + w, y, x + w, y + h, rad);
  ctx.arcTo(x + w, y + h, x, y + h, rad);
  ctx.arcTo(x, y + h, x, y, rad);
  ctx.arcTo(x, y, x + w, y, rad);
  ctx.closePath();
}

// Cross-DB ids collide — composite `${namespace}::${id}` keeps layout/selection
// unambiguous. (Ported from the previous React Flow renderer.)
const keyOf = (n: { id: string; namespace?: string }) => `${n.namespace ?? ""}::${n.id}`;

const nodeCaption = (n: { id: string; properties: Record<string, unknown> }): string => {
  const p = n.properties ?? {};
  for (const key of ["name", "title", "full_name", "path", "label"]) {
    const v = p[key];
    if (typeof v === "string" && v.trim()) return v.trim();
  }
  return n.id.split("::").pop() ?? n.id;
};

const edgeEndpointKey = (
  e: { source_id: string; target_id: string; namespace?: string; properties?: Record<string, unknown> },
  end: "source_id" | "target_id",
) => {
  const ns =
    end === "target_id"
      ? ((e.properties?.target_namespace as string | undefined) ?? e.namespace ?? "")
      : (e.namespace ?? "");
  return `${ns}::${e[end]}`;
};

type FNode = NodeObject & {
  id: string;
  label: string;
  labels: string[];
  color: string;
  deg: number;
  community: number;
  landmark: boolean;
  icon: ResolvedIcon | null;
};
type FLink = LinkObject & {
  relType: string;
  famColor: string;
  weight: number;
};

function useSize<T extends HTMLElement>() {
  const ref = useRef<T | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    if (!ref.current) return;
    const el = ref.current;
    const ro = new ResizeObserver(() => {
      setSize({ width: el.clientWidth, height: el.clientHeight });
    });
    ro.observe(el);
    setSize({ width: el.clientWidth, height: el.clientHeight });
    return () => ro.disconnect();
  }, []);
  return { ref, size };
}

export function GraphCanvas({
  nodes,
  edges,
  layout,
  showEdgeLabels,
  onNodeClick,
  onNodeHover,
  onNodeDoubleClick,
}: GraphCanvasProps) {
  const fgRef = useRef<ForceGraphMethods | undefined>(undefined);
  const { ref: containerRef, size } = useSize<HTMLDivElement>();
  const isDark = useTheme((s) => s.resolved === "dark");

  // Fit the view, then clamp over-zoom — a single/sparse graph would otherwise
  // zoom so far that one glowing node fills the whole screen.
  const fitView = useCallback((duration = 400) => {
    const fg = fgRef.current;
    if (!fg) return;
    fg.zoomToFit(duration, 90);
    window.setTimeout(() => {
      const z = fg.zoom?.() ?? 1;
      if (z > MAX_FIT_ZOOM) fg.zoom(MAX_FIT_ZOOM, 220);
    }, duration + 70);
  }, []);

  // Repaint once an icon image finishes decoding (the engine may be paused).
  const requestRefresh = useCallback(() => {
    (fgRef.current as unknown as { refresh?: () => void } | undefined)?.refresh?.();
  }, []);

  // Style toggles + interaction state from the store.
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const hoveredNodeId = useGraphStore((s) => s.hoveredNodeId);
  const searchQuery = useGraphStore((s) => s.searchQuery);
  const labelFilter = useGraphStore((s) => s.labelFilter);
  const relTypeFilter = useGraphStore((s) => s.relTypeFilter);
  const sizeByDegree = useGraphStore((s) => s.sizeByDegree);
  const glow = useGraphStore((s) => s.glow);
  const animatedFlow = useGraphStore((s) => s.animatedFlow);
  const curvedEdges = useGraphStore((s) => s.curvedEdges);
  const communityHulls = useGraphStore((s) => s.communityHulls);

  // ── Build composite-id graph data (structural; rebuilt only on data change) ──
  const data = useMemo(() => {
    const compNodes = nodes.map((n) => ({ ...n, _cid: keyOf(n) }));
    const idSet = new Set(compNodes.map((n) => n._cid));
    const compEdges = edges
      .map((e) => ({
        ...e,
        _src: edgeEndpointKey(e, "source_id"),
        _dst: edgeEndpointKey(e, "target_id"),
      }))
      .filter((e) => idSet.has(e._src) && idSet.has(e._dst));

    // Degree centrality.
    const deg = new Map<string, number>();
    for (const e of compEdges) {
      deg.set(e._src, (deg.get(e._src) ?? 0) + 1);
      deg.set(e._dst, (deg.get(e._dst) ?? 0) + 1);
    }
    const degVals = [...deg.values()].sort((a, b) => a - b);
    const capDeg = degVals.length ? degVals[Math.floor(degVals.length * 0.95)] || 1 : 1;
    const landmarkThreshold = Math.max(4, capDeg * 0.6);

    // Communities (for hull overlays / stable tinting).
    const community = detectCommunities(
      compNodes.map((n) => n._cid),
      compEdges.map((e) => ({ source: e._src, target: e._dst })),
    );

    // Adjacency for hover highlight.
    const adj = new Map<string, Set<string>>();
    for (const e of compEdges) {
      (adj.get(e._src) ?? adj.set(e._src, new Set()).get(e._src)!).add(e._dst);
      (adj.get(e._dst) ?? adj.set(e._dst, new Set()).get(e._dst)!).add(e._src);
    }

    const gnodes: FNode[] = compNodes.map((n) => {
      const d = deg.get(n._cid) ?? 0;
      return {
        id: n._cid,
        label: nodeCaption(n),
        labels: n.labels,
        color: resolveNodeColor(n.labels, n.properties),
        deg: d,
        community: community.get(n._cid) ?? 0,
        landmark: d >= landmarkThreshold,
        icon: resolveGraphIcon(n.labels, n.properties),
      };
    });
    const glinks: FLink[] = compEdges.map((e) => ({
      source: e._src,
      target: e._dst,
      relType: e.relationship_type,
      famColor: getRelationshipFamilyColor(e.relationship_type),
      weight: Math.min(deg.get(e._src) ?? 1, deg.get(e._dst) ?? 1),
    }));

    return { nodes: gnodes, links: glinks, capDeg, adj };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodes, edges]);

  // Precompute positions with kyma's tuned layout algorithms for EVERY layout
  // (including "force" — the organic spread cloud) and pin them. Passing the
  // full nodes (with `labels`) lets force/tree grouping work. We rely on these
  // deterministic, well-distributed positions rather than react-force-graph's
  // default physics, which collapses dense graphs into an unreadable knot.
  useEffect(() => {
    const idOf = (e: string | number | { id?: string | number } | undefined) =>
      (typeof e === "object" ? String(e?.id ?? "") : String(e ?? ""));
    const pos = computeLayout(
      layout,
      data.nodes as unknown as { id: string }[],
      data.links.map((l) => ({ source_id: idOf(l.source), target_id: idOf(l.target) })),
      1600,
      1000,
    );
    for (const n of data.nodes) {
      const p = pos.get(n.id);
      if (p) {
        n.x = p.x;
        n.y = p.y;
        (n as FNode).fx = p.x;
        (n as FNode).fy = p.y;
      }
    }
    fgRef.current?.d3ReheatSimulation?.();
    const t = setTimeout(() => fitView(500), 90);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layout, data]);

  // ── Focus highlight: selected/hovered node + its neighbours ──
  const focusId = selectedNodeId ?? hoveredNodeId;
  const focusSet = useMemo(() => {
    if (!focusId) return null;
    const s = new Set<string>([focusId]);
    for (const nb of data.adj.get(focusId) ?? []) s.add(nb);
    return s;
  }, [focusId, data.adj]);

  const query = searchQuery.trim().toLowerCase();
  const matchesSearch = useCallback(
    (n: FNode) => !!query && (n.label.toLowerCase().includes(query) || n.id.toLowerCase().includes(query)),
    [query],
  );
  const isDimmed = useCallback(
    (n: FNode) => {
      if (labelFilter) return !n.labels.includes(labelFilter);
      if (relTypeFilter) return false; // rel filter handled on links
      if (query) return !matchesSearch(n);
      if (focusSet) return !focusSet.has(n.id);
      return false;
    },
    [labelFilter, relTypeFilter, query, focusSet, matchesSearch],
  );

  // Re-emit particles / repaint when focus changes (engine may be paused).
  useEffect(() => {
    (fgRef.current as unknown as { refresh?: () => void } | undefined)?.refresh?.();
  }, [focusId, animatedFlow, glow, curvedEdges, sizeByDegree, communityHulls, query, labelFilter, relTypeFilter, showEdgeLabels]);

  // Gentle reposition on selection: KEEP the current zoom, and only pan when
  // the node is outside the comfortable central region — no jarring zoom snap
  // or recenter when you click a node that's already in view.
  useEffect(() => {
    if (!selectedNodeId) return;
    const fg = fgRef.current;
    if (!fg) return;
    const n = data.nodes.find((x) => x.id === selectedNodeId) as FNode | undefined;
    if (!n || n.x == null || n.y == null) return;
    const w = size.width;
    const h = size.height;
    const toScreen = (
      fg as unknown as {
        graph2ScreenCoords?: (x: number, y: number) => { x: number; y: number };
      }
    ).graph2ScreenCoords;
    if (toScreen && w && h) {
      const sc = toScreen(n.x, n.y);
      // Already comfortably framed → leave the camera exactly where it is.
      if (Math.abs(sc.x - w / 2) < w * 0.33 && Math.abs(sc.y - h / 2) < h * 0.33) return;
    }
    fg.centerAt(n.x, n.y, 600);
  }, [selectedNodeId, data.nodes, size.width, size.height]);

  // ── Node paint ──
  const paintNode = useCallback(
    (node: NodeObject, ctx: CanvasRenderingContext2D, globalScale: number) => {
      const n = node as FNode;
      if (n.x == null || n.y == null) return;
      const selected = n.id === selectedNodeId;
      const hovered = n.id === hoveredNodeId;
      const search = matchesSearch(n);
      const dim = isDimmed(n) && !selected && !hovered;
      const r = radiusForDegree(n.deg, data.capDeg, sizeByDegree) * (selected ? 1.3 : search ? 1.18 : 1);
      const screenR = r * globalScale;
      const g0 = Math.max(globalScale, 0.6);

      ctx.save();
      ctx.globalAlpha = dim ? 0.1 : 1;
      ctx.lineJoin = "round";

      // ── Depth: a colored glow on focus/landmark nodes, a soft ambient drop
      // shadow on resting ones — the field reads calm and dimensional, not flat.
      const wantGlow = glow && (selected || hovered || search || n.landmark);
      if (!dim) {
        if (wantGlow) {
          ctx.shadowColor = search ? "#f59e0b" : n.color;
          // Softer glow than before — present but not blooming.
          const sb = selected ? 11 : hovered ? 8 : search ? 9 : 4;
          ctx.shadowBlur = sb / g0;
        } else if (screenR >= 4) {
          ctx.shadowColor = "rgba(2,6,14,0.5)";
          ctx.shadowBlur = 3.5 / g0;
          ctx.shadowOffsetY = 1 / g0;
        }
      }

      // ── Orb fill: off-center highlight → base → slightly darker rim, for a
      // soft spherical read instead of a flat disc.
      const grad = ctx.createRadialGradient(
        n.x - r * 0.36, n.y - r * 0.42, r * 0.1,
        n.x, n.y, r * 1.06,
      );
      grad.addColorStop(0, lighten(n.color, 0.45));
      grad.addColorStop(0.55, n.color);
      grad.addColorStop(1, darken(n.color, 0.22));
      ctx.beginPath();
      ctx.arc(n.x, n.y, r, 0, 2 * Math.PI);
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.shadowOffsetY = 0;

      // ── Hairline rim — crisp definition against the canvas and neighbours.
      ctx.lineWidth = 1 / globalScale;
      ctx.strokeStyle = darken(n.color, 0.42);
      ctx.stroke();

      // ── Focus ring (selected / search): a thin bright ring offset just
      // outside the rim.
      if (selected || search) {
        ctx.beginPath();
        ctx.arc(n.x, n.y, r + 2.4 / globalScale, 0, 2 * Math.PI);
        ctx.lineWidth = (selected ? 1.6 : 1.3) / globalScale;
        ctx.strokeStyle = search ? "#f59e0b" : isDark ? "#e8eef6" : "#0f172a";
        ctx.stroke();
      }

      // ── Type / vendor icon, once the node is big enough to read it.
      //  • Brand marks (github, datadog, k8s, …) are intricate, so they sit on a
      //    clean white chip in a dark glyph — recognisable instead of a blob.
      //  • Kind glyphs (service, table, …) draw as a soft white glyph directly
      //    on the colored orb.
      // Icons appear only once a node is large enough on screen to read — so
      // the zoomed-out view stays clean, color-coded orbs.
      if (n.icon && !dim && screenR >= 7) {
        if (n.icon.brand) {
          // Brand mark on a clean white chip, in its TRUE brand color — the
          // recognisable way to show github / datadog / k8s inside a colored orb.
          const chipR = r * 0.66;
          ctx.beginPath();
          ctx.arc(n.x, n.y, chipR, 0, 2 * Math.PI);
          ctx.fillStyle = "#ffffff";
          ctx.fill();
          ctx.lineWidth = 0.75 / globalScale;
          ctx.strokeStyle = "rgba(15,23,42,0.16)";
          ctx.stroke();
          const img = getIconImage(n.icon, "default", requestRefresh);
          if (img.complete && img.naturalWidth > 0) {
            const s = chipR * 1.5;
            ctx.drawImage(img, n.x - s / 2, n.y - s / 2, s, s);
          }
        } else {
          const img = getIconImage(n.icon, "rgba(255,255,255,0.95)", requestRefresh);
          if (img.complete && img.naturalWidth > 0) {
            const s = r * 1.3;
            ctx.shadowColor = "rgba(0,0,0,0.35)";
            ctx.shadowBlur = 2 / globalScale;
            ctx.drawImage(img, n.x - s / 2, n.y - s / 2, s, s);
            ctx.shadowBlur = 0;
          }
        }
      }

      // ── Label: refined type. A soft rounded pill behind emphasized labels for
      // legibility; plain shadowed text for the rest.
      const showLabel =
        !dim &&
        (globalScale >= LOD.labelAll ||
          ((selected || hovered || search || n.landmark) && globalScale >= LOD.labelLandmark));
      if (showLabel) {
        const fontSize = Math.min(13, Math.max(9, 11 / globalScale + 2));
        ctx.font = `${selected ? 600 : 500} ${fontSize}px "IBM Plex Sans", ui-sans-serif, sans-serif`;
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        const label = n.label.length > 30 ? n.label.slice(0, 29) + "…" : n.label;
        const ly = n.y + r + 4 / globalScale;
        if (selected || hovered || search) {
          const tw = ctx.measureText(label).width;
          const padX = 5 / globalScale;
          const padY = 2.4 / globalScale;
          const w = tw + padX * 2;
          const h = fontSize + padY * 2;
          ctx.fillStyle = isDark ? "rgba(13,18,26,0.74)" : "rgba(255,255,255,0.86)";
          roundRectPath(ctx, n.x - w / 2, ly - padY, w, h, 4 / globalScale);
          ctx.fill();
          ctx.fillStyle = search
            ? "#f59e0b"
            : selected
              ? (isDark ? "#f8fafc" : "#0f172a")
              : (isDark ? "#e2e8f0" : "#0f172a");
          ctx.fillText(label, n.x, ly);
        } else {
          ctx.fillStyle = isDark ? "rgba(203,213,225,0.92)" : "rgba(30,41,59,0.92)";
          ctx.shadowColor = isDark ? "rgba(0,0,0,0.85)" : "rgba(255,255,255,0.9)";
          ctx.shadowBlur = 3;
          ctx.fillText(label, n.x, ly);
          ctx.shadowBlur = 0;
        }
      }
      ctx.restore();
    },
    [selectedNodeId, hoveredNodeId, matchesSearch, isDimmed, sizeByDegree, glow, isDark, data.capDeg, requestRefresh],
  );

  // Pointer hit area sized to the node.
  const paintPointer = useCallback(
    (node: NodeObject, color: string, ctx: CanvasRenderingContext2D) => {
      const n = node as FNode;
      if (n.x == null || n.y == null) return;
      const r = radiusForDegree(n.deg, data.capDeg, sizeByDegree) + 2;
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(n.x, n.y, r, 0, 2 * Math.PI);
      ctx.fill();
    },
    [data.capDeg, sizeByDegree],
  );

  // ── Link styling ──
  const linkColor = useCallback(
    (link: LinkObject) => {
      const l = link as FLink;
      if (relTypeFilter && l.relType !== relTypeFilter) return alpha(l.famColor, 0.04);
      if (focusSet) {
        const s = typeof l.source === "object" ? (l.source as FNode).id : (l.source as string);
        const t = typeof l.target === "object" ? (l.target as FNode).id : (l.target as string);
        const incident = s === focusId || t === focusId;
        return incident ? alpha(l.famColor, 0.95) : alpha(l.famColor, 0.05);
      }
      return alpha(l.famColor, isDark ? 0.38 : 0.5);
    },
    [relTypeFilter, focusSet, focusId, isDark],
  );

  const linkWidth = useCallback(
    (link: LinkObject) => {
      const l = link as FLink;
      const base = 0.6 + Math.min(l.weight, 8) * 0.18;
      if (focusSet) {
        const s = typeof l.source === "object" ? (l.source as FNode).id : (l.source as string);
        const t = typeof l.target === "object" ? (l.target as FNode).id : (l.target as string);
        return s === focusId || t === focusId ? base + 0.8 : base * 0.6;
      }
      return base;
    },
    [focusSet, focusId],
  );

  const linkParticles = useCallback(
    (link: LinkObject) => {
      if (!animatedFlow || !focusSet) return 0;
      const l = link as FLink;
      const s = typeof l.source === "object" ? (l.source as FNode).id : (l.source as string);
      const t = typeof l.target === "object" ? (l.target as FNode).id : (l.target as string);
      return s === focusId || t === focusId ? 3 : 0;
    },
    [animatedFlow, focusSet, focusId],
  );

  // ── Community hull overlay (drawn beneath nodes each frame) ──
  const onRenderFramePre = useCallback(
    (ctx: CanvasRenderingContext2D, globalScale: number) => {
      if (!communityHulls) return;
      const groups = new Map<number, FNode[]>();
      for (const n of data.nodes as FNode[]) {
        if (n.x == null || n.y == null) continue;
        (groups.get(n.community) ?? groups.set(n.community, []).get(n.community)!).push(n);
      }
      for (const [cid, members] of groups) {
        if (members.length < 4) continue;
        const hull = padHull(
          convexHull(members.map((m) => ({ x: m.x!, y: m.y! }))),
          22,
        );
        if (hull.length < 3) continue;
        const color = paletteFor(cid);
        ctx.beginPath();
        ctx.moveTo(hull[0].x, hull[0].y);
        for (let i = 1; i < hull.length; i++) ctx.lineTo(hull[i].x, hull[i].y);
        ctx.closePath();
        ctx.fillStyle = alpha(color, 0.06);
        ctx.fill();
        ctx.lineWidth = 1.5 / globalScale;
        ctx.strokeStyle = alpha(color, 0.22);
        ctx.stroke();
      }
    },
    [communityHulls, data.nodes],
  );

  const bg = isDark ? "rgba(0,0,0,0)" : "rgba(0,0,0,0)";

  return (
    <div ref={containerRef} className="absolute inset-0">
      <ForceGraph2D
        ref={fgRef as never}
        width={size.width || undefined}
        height={size.height || undefined}
        graphData={{ nodes: data.nodes, links: data.links } as never}
        backgroundColor={bg}
        nodeRelSize={4}
        nodeCanvasObject={paintNode}
        nodePointerAreaPaint={paintPointer}
        linkColor={linkColor}
        linkWidth={linkWidth}
        linkCurvature={curvedEdges ? 0.18 : 0}
        linkDirectionalParticles={linkParticles}
        linkDirectionalParticleWidth={2}
        linkDirectionalParticleSpeed={0.006}
        linkDirectionalParticleColor={(l) => (l as FLink).famColor}
        linkLabel={showEdgeLabels ? (l) => (l as FLink).relType : undefined}
        onRenderFramePre={onRenderFramePre}
        cooldownTicks={0}
        minZoom={0.06}
        maxZoom={8}
        onEngineStop={() => fitView(400)}
        onNodeClick={(n: FNode) => onNodeClick(n.id)}
        onNodeHover={(n: FNode | null) => onNodeHover(n ? n.id : null)}
        onNodeDragEnd={(n: FNode) => {
          n.fx = n.x;
          n.fy = n.y;
        }}
        onBackgroundClick={() => onNodeClick("")}
        onNodeRightClick={(n: FNode) => onNodeDoubleClick?.(n.id)}
        autoPauseRedraw={false}
      />
      <CanvasControls
        onZoomIn={() => fgRef.current?.zoom((fgRef.current?.zoom() ?? 1) * 1.4, 250)}
        onZoomOut={() => fgRef.current?.zoom((fgRef.current?.zoom() ?? 1) / 1.4, 250)}
        onFit={() => fitView(400)}
      />
    </div>
  );
}

function CanvasControls({
  onZoomIn,
  onZoomOut,
  onFit,
}: {
  onZoomIn: () => void;
  onZoomOut: () => void;
  onFit: () => void;
}) {
  const btn =
    "flex h-8 w-8 items-center justify-center text-muted-foreground transition-colors hover:bg-accent hover:text-foreground";
  return (
    <div className="glass absolute bottom-4 left-4 flex flex-col overflow-hidden rounded-lg">
      <button type="button" className={btn} onClick={onZoomIn} title="Zoom in" aria-label="Zoom in">
        <Plus className="h-4 w-4" />
      </button>
      <button type="button" className={`${btn} border-y border-border/60`} onClick={onFit} title="Fit to view" aria-label="Fit to view">
        <Maximize2 className="h-3.5 w-3.5" />
      </button>
      <button type="button" className={btn} onClick={onZoomOut} title="Zoom out" aria-label="Zoom out">
        <Minus className="h-4 w-4" />
      </button>
    </div>
  );
}
