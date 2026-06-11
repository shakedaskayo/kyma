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
import { EdgeArrowProgram } from "sigma/rendering";
import type { GraphNode, GraphRelationship } from "@kyma-ai/client";
import { useKymaContext } from "../provider/context";
import { useGraphStore } from "./graph-store";
import { edgeDisplay, lodTier, nodeDisplay, type DisplayCtx } from "./graph-display";
import { alpha as withAlpha } from "./graph-style";
import { buildGraphologyGraph } from "./sigma-graph-builder";
// Re-export builder, BuildOptions, and keyOf for tests and consumers.
export { buildGraphologyGraph, keyOf } from "./sigma-graph-builder";
export type { BuildOptions } from "./sigma-graph-builder";

// ── Neutral edge color helper ──────────────────────────────────────────────

/** RGB portion of the neutral hairline color. */
const NEUTRAL_RGB = "148,163,184";

function neutralEdgeColor(alpha: number): string {
  return `rgba(${NEUTRAL_RGB},${alpha})`;
}

// ── Component props ────────────────────────────────────────────────────────

export interface SigmaCanvasProps {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  positions: Map<string, { x: number; y: number }>;
  /** Bump to rebuild the graphology instance (export accumulator version). */
  version: number;
  activeNamespaces: Set<string>;
  /** Focus-neighborhood isolation root (double-click). Null = off. Task 9 moves this to store. */
  focusModeId: string | null;
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
  const { isDark } = useKymaContext();

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
    const { focusModeId } = props;
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
    props.focusModeId,
  ]);

  // Instantiate Sigma once per graph + style settings.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const sigma = new Sigma(graph, container, {
      defaultEdgeType: curvedEdges ? "curvedArrow" : "straightArrow",
      edgeProgramClasses: {
        straightArrow: EdgeArrowProgram,
        curvedArrow: EdgeCurvedArrowProgram,
      },
      nodeProgramClasses: {
        image: createNodeImageProgram({ keepWithinCircle: true }),
      },
      renderEdgeLabels: true,
      labelColor: { color: isDark ? "#cbd5e1" : "#334155" },
      edgeLabelColor: { color: isDark ? "#94a3b8" : "#64748b" },
      labelFont: "IBM Plex Sans, sans-serif",
      edgeLabelFont: "JetBrains Mono, monospace",
      labelSize: 11,
      edgeLabelSize: 9,
      minCameraRatio: 0.02,
      maxCameraRatio: 4,
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

        return {
          ...attrs,
          type: useImageType ? "image" : "circle",
          hidden: d.hidden,
          label: d.label,
          color: d.dimmed
            ? withAlpha(attrs.color as string, 0.15)
            : (attrs.color as string),
          size: d.highlighted
            ? (attrs.size as number) * 1.25
            : (attrs.size as number),
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

        // Amendment: at-rest (neutral) edges use slate hairline, not family color.
        const color = d.neutral
          ? neutralEdgeColor(d.alpha)
          : withAlpha(attrs.familyColor as string, d.alpha);

        return {
          ...attrs,
          hidden: d.hidden,
          color,
          size: d.size,
          label: d.label,
          zIndex: d.loud ? 2 : 0,
        };
      },
    });

    sigmaRef.current = sigma;
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

    // LOD: refresh reducers on every camera move so tier updates propagate.
    // Update ctxRef.current.tier BEFORE refreshing so reducers see the current
    // LOD tier — without this the tier lags one frame (computed at React render
    // time, not at camera-update time).
    sigma.getCamera().on("updated", () => {
      ctxRef.current.tier = lodTier(sigma.getCamera().ratio);
      sigma.refresh({ skipIndexation: true });
    });

    return () => {
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
    props.focusModeId,
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

  return <div ref={containerRef} className="ky-absolute ky-inset-0" />;
}
