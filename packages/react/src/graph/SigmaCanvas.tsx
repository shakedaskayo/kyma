/**
 * SigmaCanvas — WebGL renderer for the full graph. Replaces GraphCanvas on
 * WebGL-capable browsers. Nodes/edges/positions come fully loaded from
 * useGraphExport; this component renders ALL of them and lets the
 * quiet/loud reducers + LOD carry legibility.
 *
 * Amendment (2026-06-11):
 * - Nodes render as plain circles unless mid/near LOD or isLandmark (top decile).
 * - Node radius rescaled to [2.5, 22] so hubs visually dominate.
 * - At-rest edges use neutral hairline rgba(148,163,184,alpha), not family color.
 */
import { useEffect, useMemo, useRef } from "react";
import Sigma from "sigma";
import { createNodeImageProgram } from "@sigma/node-image";
import { EdgeCurvedArrowProgram } from "@sigma/edge-curve";
import { EdgeArrowProgram, EdgeLineProgram } from "sigma/rendering";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/client";
import { usePensieveContext } from "../provider/context";
import { useGraphStore } from "./graph-store";
import { edgeDisplay, lodTier, nodeDisplay, type DisplayCtx } from "./graph-display";
import { makeDrawNodeHover } from "./graph-hover-draw";
import { alpha as withAlpha } from "./graph-style";
import { buildGraphologyGraph, positionExtent, RADIUS_DST_MAX } from "./sigma-graph-builder";
// Re-export builder, BuildOptions, and keyOf for tests and consumers.
export { buildGraphologyGraph, keyOf } from "./sigma-graph-builder";
export type { BuildOptions } from "./sigma-graph-builder";

// ── Neutral edge color helper ──────────────────────────────────────────────

/** Neutral hairline color per theme — slate-400 on dark, slate-600 on light
 *  (slate-400 on a white canvas is nearly invisible). */
const NEUTRAL_HEX_DARK = "#94a3b8";
const NEUTRAL_HEX_LIGHT = "#475569";

/** Approximate galaxy-substrate colors the edges sit on, per theme. */
const CANVAS_BG_DARK = "#0c1322";
const CANVAS_BG_LIGHT = "#f4f6f9";

/**
 * Pre-baked translucency: blend `fg` toward the theme background by `t` and
 * return an OPAQUE hex. Sigma's edge programs wash translucent colors out
 * toward white (blend-convention mismatch), so we never hand them alpha —
 * opaque colors render identically everywhere, and overlapping edges no
 * longer stack to white-hot at hubs.
 */
function mixToBg(fgHex: string, t: number, isDark: boolean): string {
  const bgHex = isDark ? CANVAS_BG_DARK : CANVAS_BG_LIGHT;
  const p = (h: string) => parseInt(h.replace("#", ""), 16);
  const f = p(fgHex);
  const b = p(bgHex);
  const ch = (shift: number) =>
    Math.round(((b >> shift) & 255) * (1 - t) + ((f >> shift) & 255) * t);
  return `#${((ch(16) << 16) | (ch(8) << 8) | ch(0)).toString(16).padStart(6, "0")}`;
}

/** Below this node count, screen-pixel sizing beats the constellation look. */
const CONSTELLATION_MIN_NODES = 300;

/** Up to this many edges, at-rest edges keep small directional arrowheads —
 *  beyond it, arrow programs cost too much and direction arrives on focus. */
const REST_ARROWS_MAX_EDGES = 5_000;

/** Map builder sizes [2.5,22] into a comfortable screen-px range [6,18]. */
function screenSize(size: number): number {
  return 6 + ((size - 2.5) / (22 - 2.5)) * 12;
}

function neutralEdgeColor(alpha: number, isDark: boolean): string {
  return mixToBg(isDark ? NEUTRAL_HEX_DARK : NEUTRAL_HEX_LIGHT, alpha, isDark);
}

// ── Component props ────────────────────────────────────────────────────────

export interface SigmaCanvasProps {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  positions: Map<string, { x: number; y: number }>;
  /** Bump to rebuild the graphology instance (export accumulator version). */
  version: number;
  activeNamespaces: Set<string>;
  onNodeClick: (compositeId: string) => void;
  onNodeHover: (compositeId: string | null) => void;
  onNodeDoubleClick: (compositeId: string) => void;
  /** Receives the live Sigma instance (for minimap / fly-to / keyboard). */
  onSigmaReady?: (sigma: Sigma) => void;
}

// ── Component ──────────────────────────────────────────────────────────────

export function SigmaCanvas(props: SigmaCanvasProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const sigmaRef = useRef<Sigma | null>(null);
  // Camera continuity across per-chunk sigma rebuilds (see restore below).
  const savedCameraRef = useRef<{ x: number; y: number; ratio: number; angle: number } | null>(null);
  const userMovedCameraRef = useRef(false);
  const { isDark } = usePensieveContext();

  // Store state.
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

  // Build the graphology graph when data or relevant options change.
  // `version` acts as the data-change signal (the accumulator is mutable).
  const graph = useMemo(
    () =>
      buildGraphologyGraph(props.nodes, props.edges, props.positions, {
        sizeByDegree,
        hiddenLabels,
        activeNamespaces: props.activeNamespaces,
      }),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [props.version, sizeByDegree, hiddenLabels, props.activeNamespaces],
  );

  // Display context — rebuilt on interaction changes, read by reducers each frame.
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
        if (
          String(attrs.label).toLowerCase().includes(q) ||
          id.toLowerCase().includes(q)
        ) {
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
  }, [
    graph,
    hoveredNodeId,
    selectedNodeId,
    searchQuery,
    relTypeFilter,
    showEdgeLabels,
    isDark,
    focusModeId,
  ]);

  // Instantiate Sigma once per graph + style settings.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    // Dense graphs stack thousands of translucent hairlines per pixel — scale
    // the at-rest alpha down with edge count so fans stay airy, never a wash.
    const edgeDensityScale = Math.max(0.05, Math.min(1, 9_000 / Math.max(1, graph.size)));

    // Constellation mode: graph-space sizing only pays off for big graphs
    // with a real spatial extent; small graphs read better at fixed px sizes.
    const constellation =
      graph.order >= CONSTELLATION_MIN_NODES &&
      positionExtent(graph) >= 4 * RADIUS_DST_MAX;
    const restArrows = graph.size <= REST_ARROWS_MAX_EDGES;

    const sigma = new Sigma(graph, container, {
      defaultEdgeType: "line",
      edgeProgramClasses: {
        // GL_LINES hairlines for the at-rest neutral mass — orders of magnitude
        // cheaper than arrow/curve programs at 100k edges. Loud edges switch
        // to arrow programs via the reducer's `type` override.
        line: EdgeLineProgram,
        straightArrow: EdgeArrowProgram,
        curvedArrow: EdgeCurvedArrowProgram,
      },
      nodeProgramClasses: {
        // Reference look: solid label-colored disc with a padded white glyph
        // inside ("background" composites the icon over the node color).
        image: createNodeImageProgram({
          drawingMode: "background",
          keepWithinCircle: true,
          padding: 0.22,
        }),
      },
      renderEdgeLabels: true,
      defaultDrawNodeHover: makeDrawNodeHover(isDark),
      labelColor: { color: isDark ? "#cbd5e1" : "#334155" },
      edgeLabelColor: { color: isDark ? "#94a3b8" : "#64748b" },
      labelFont: "IBM Plex Sans, sans-serif",
      edgeLabelFont: "JetBrains Mono, monospace",
      labelSize: 11,
      edgeLabelSize: 9,
      minCameraRatio: 0.02,
      maxCameraRatio: 4,
      // Sizing strategy by graph scale:
      // - LARGE graphs (≥ CONSTELLATION_MIN_NODES, sane extent): sizes live in
      //   graph-space ("positions"), so zooming out shrinks dots into the
      //   constellation look instead of constant-pixel blobs.
      // - SMALL or degenerate graphs (few nodes, or coincident positions where
      //   autoRescale would explode graph-space sizes to fill the viewport):
      //   classic screen-pixel sizing, with sizes lifted into a comfortable
      //   visible range by the node reducer below.
      ...(constellation
        ? {
            itemSizesReference: "positions" as const,
            zoomToSizeRatioFunction: (ratio: number) => ratio,
          }
        : {}),
      // Reducers own label visibility — disable sigma's threshold-based culling.
      labelRenderedSizeThreshold: 0,
      zIndex: true,
      nodeReducer: (id, attrs) => {
        const d = nodeDisplay(id, attrs as { label: string; size: number }, ctxRef.current);
        const tier = ctxRef.current.tier;
        const isLandmark = Boolean(attrs.isLandmark);
        const hasImage = Boolean(attrs.image);

        // Amendment: use image type only at mid/near LOD or for landmark hubs.
        const useImageType = hasImage && (tier !== "far" || isLandmark);

        const baseSize = constellation
          ? (attrs.size as number)
          : screenSize(attrs.size as number);

        // Tiny graphs: always label visible nodes — LOD tiers are meaningless
        // when the whole graph fits in a glance.
        const label =
          d.label ?? (!d.hidden && !d.dimmed && graph.order <= 50 ? (attrs.label as string) : null);

        return {
          ...attrs,
          type: useImageType ? "image" : "circle",
          hidden: d.hidden,
          label,
          color: d.dimmed
            ? withAlpha(attrs.color as string, 0.15)
            : (attrs.color as string),
          size: d.highlighted ? baseSize * 1.25 : baseSize,
          zIndex: d.highlighted ? 2 : d.dimmed ? 0 : 1,
          highlighted: d.highlighted,
        };
      },
      edgeReducer: (id, attrs) => {
        // `graph` (not `sigma.getGraph()`) — Sigma's constructor runs reducers
        // synchronously, while the `sigma` const is still in its TDZ.
        const [src, tgt] = graph.extremities(id);
        const d = edgeDisplay(
          id,
          attrs as { size: number; relType: string },
          src,
          tgt,
          ctxRef.current,
        );

        // Amendment: at-rest (neutral) edges use slate hairline, not family
        // color. All edge colors are pre-baked opaque (see mixToBg).
        const color = d.neutral
          ? neutralEdgeColor(d.alpha * edgeDensityScale, isDark)
          : mixToBg(attrs.familyColor as string, d.alpha, isDark);

        // Quiet edges keep small directional arrowheads while the arrow
        // program is affordable; huge graphs fall back to plain GL lines and
        // direction arrives in the loud (focus) state.
        const arrowType = curvedEdges ? "curvedArrow" : "straightArrow";
        const restType = restArrows ? arrowType : "line";

        // In constellation mode sizes live in graph-space and grow as you zoom
        // in — fine for nodes, but edges must stay hairlines, not fat bars.
        // Multiplying by the camera ratio keeps their on-screen width constant
        // (reducers re-run on every camera update).
        const zoomClamp = constellation
          ? (sigmaRef.current?.getCamera().ratio ?? 1)
          : 1;

        // At-rest arrow edges get a touch more width so the arrowheads
        // actually read as direction markers (heads scale with edge size).
        const restWidth = restArrows && !d.loud ? Math.max(d.size, 1.6) : d.size;

        return {
          ...attrs,
          type: d.loud ? arrowType : restType,
          hidden: d.hidden,
          color,
          size: restWidth * zoomClamp,
          label: d.label,
          zIndex: d.loud ? 2 : 0,
        };
      },
    });

    sigmaRef.current = sigma;
    // Restore the user's camera across chunk-streaming rebuilds — each new
    // Sigma instance starts at the fitted default, which would yank the view
    // back while pages arrive. Only restore once the user has actually moved.
    if (userMovedCameraRef.current && savedCameraRef.current) {
      sigma.getCamera().setState(savedCameraRef.current);
    }
    props.onSigmaReady?.(sigma);

    // Event wiring.
    sigma.on("clickNode", ({ node }) => props.onNodeClick(node));
    sigma.on("doubleClickNode", ({ node, event }) => {
      event.preventSigmaDefault();
      props.onNodeDoubleClick(node);
    });
    sigma.on("clickStage", () => props.onNodeClick(""));
    sigma.on("enterNode", ({ node }) => props.onNodeHover(node));
    sigma.on("leaveNode", () => props.onNodeHover(null));
    // leaveNode doesn't fire when the pointer exits the whole container (e.g.
    // straight onto the sidebar) — without this the hover-dim state sticks.
    const clearHover = () => props.onNodeHover(null);
    container.addEventListener("mouseleave", clearHover);

    // Sigma only listens to window resize, but the container also shrinks/
    // grows when the inspector panel docks/undocks beside the canvas.
    const ro = new ResizeObserver(() => sigma.resize());
    ro.observe(container);

    // LOD: refresh reducers on every camera move so tier updates propagate.
    // Update ctxRef.current.tier BEFORE refreshing so reducers see the current
    // LOD tier — without this the tier lags one frame (computed at React render
    // time, not at camera-update time).
    sigma.getCamera().on("updated", () => {
      ctxRef.current.tier = lodTier(sigma.getCamera().ratio);
      savedCameraRef.current = sigma.getCamera().getState();
      sigma.refresh({ skipIndexation: true });
    });
    // Any direct interaction marks the camera as user-owned.
    sigma.getMouseCaptor().on("mousedown", () => {
      userMovedCameraRef.current = true;
    });
    sigma.getMouseCaptor().on("wheel", () => {
      userMovedCameraRef.current = true;
    });

    return () => {
      ro.disconnect();
      container.removeEventListener("mouseleave", clearHover);
      sigmaRef.current = null;
      sigma.kill();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [graph, curvedEdges, isDark]);

  // Re-run reducers when interaction context changes (no graph rebuild needed).
  useEffect(() => {
    sigmaRef.current?.refresh({ skipIndexation: true });
  }, [
    hoveredNodeId,
    selectedNodeId,
    searchQuery,
    relTypeFilter,
    showEdgeLabels,
    focusModeId,
  ]);

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
