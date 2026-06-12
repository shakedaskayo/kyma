import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type Sigma from "sigma";
import { Network, Plus, Minus, Maximize2, RefreshCw } from "lucide-react";
import { useGraphStore } from "./graph-store";
import { useGraphExport } from "../hooks/useGraphExport";
import { graphKey } from "../hooks/useKymaGraph";
import type { GraphNode, GraphRelationship } from "@kyma-ai/client";
import { useKymaContext } from "../provider/context";
import { SigmaCanvas } from "./SigmaCanvas";
import { GraphCanvas } from "./GraphCanvas";
import { CommandBar } from "./CommandBar";
import { InspectorPanel } from "./InspectorPanel";
import { LegendDock } from "./LegendDock";
import { BreadcrumbTrail } from "./BreadcrumbTrail";
import { Minimap } from "./Minimap";
import { HullsLayer } from "./HullsLayer";
import { useKeyboardWalk } from "./useKeyboardWalk";
import type { UseKymaGraphArgs } from "../hooks/useKymaGraph";

/**
 * Cross-database unified full-bleed graph view. Always merges every (database, graph)
 * pair into one canvas. Rendered on a WebGL/Sigma renderer (falling back to the
 * old canvas renderer when WebGL is unavailable) with a dark "galaxy" treatment:
 * community hulls, glowing centrality-sized nodes, curved relationship-family
 * edges, and animated flow on the focused neighbourhood.
 *
 * NOTE: This component must be rendered inside a GraphStoreContext provider
 * (supplied by KymaGraph). Data is loaded via useGraphExport (chunked streamed
 * pages with server-side layout positions).
 */

// ── WebGL detection ────────────────────────────────────────────────────────────

/**
 * Detect WebGL availability. Tries webgl2 then webgl; returns false on any error.
 * Exported so callers can determine the default renderer.
 */
export function webglAvailable(): boolean {
  try {
    const canvas = document.createElement("canvas");
    return !!(
      canvas.getContext("webgl2") ??
      canvas.getContext("webgl")
    );
  } catch {
    return false;
  }
}

// ── Key helpers ────────────────────────────────────────────────────────────────

const keyOf = (n: { id: string; namespace?: string }) => `${n.namespace ?? ""}::${n.id}`;

// ── Props ──────────────────────────────────────────────────────────────────────

export interface GraphViewProps {
  /** Graphs to merge; omit to load all graphs of the default database. */
  graphs?: Array<{ database?: string; graph: string }>;
  /**
   * "all-databases": discover every (database, graph) across the deployment.
   * When set, `graphs` is ignored.
   */
  discover?: "all-databases";
  realm?: string;
  /**
   * Renderer to use. "webgl" uses Sigma.js, "canvas" uses the old ForceGraph2D
   * fallback. Default: auto-detected via webglAvailable().
   */
  renderer?: "webgl" | "canvas";
  /** When false, the chrome (CommandBar, inspector, legend, breadcrumbs, minimap) is hidden. Default true. */
  showChrome?: boolean;
  /**
   * Fires with the RESOLVED selected node whenever the selection changes
   * (null on deselect). This is the public-callback bridge for KymaGraph —
   * the full GraphNode lives in this component's node map, not in the store.
   */
  onSelectedNodeChange?: (node: GraphNode | null) => void;
  /** Deep-link target: raw node id (or composite `ns::id`) to center + highlight
   *  once it's loaded. */
  focusNodeId?: string;
}

// ── Component ─────────────────────────────────────────────────────────────────

export function GraphView({
  graphs,
  discover,
  realm,
  renderer,
  showChrome = true,
  onSelectedNodeChange,
  focusNodeId,
}: GraphViewProps) {
  // ── Store state ─────────────────────────────────────────────────────────────
  const graph = useGraphStore((s) => s.graph);
  const setGraph = useGraphStore((s) => s.setGraph);
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const selectNode = useGraphStore((s) => s.selectNode);
  const focusNode = useGraphStore((s) => s.focusNode);
  const hoverNode = useGraphStore((s) => s.hoverNode);
  const hiddenLabels = useGraphStore((s) => s.hiddenLabels);
  const hiddenNamespaces = useGraphStore((s) => s.hiddenNamespaces);
  const setHiddenNamespaces = useGraphStore((s) => s.setHiddenNamespaces);
  const showEdgeLabels = useGraphStore((s) => s.showEdgeLabels);
  const layout = useGraphStore((s) => s.layout);
  const focusModeId = useGraphStore((s) => s.focusModeId);
  const setFocusMode = useGraphStore((s) => s.setFocusMode);
  const pushTrail = useGraphStore((s) => s.pushTrail);

  const { isDark } = useKymaContext();

  // ── Data loading ─────────────────────────────────────────────────────────────
  const exportArgs: UseKymaGraphArgs & { algorithm: typeof layout } = {
    graphs,
    discover,
    realm,
    algorithm: layout,
  };
  const exp = useGraphExport(exportArgs);

  // Destructure accumulator fields.
  const nodes: GraphNode[] = exp.acc.nodes;
  const edges: GraphRelationship[] = exp.acc.edges;

  // ── Namespace management ─────────────────────────────────────────────────────
  const coords = exp.coords;
  const namespaceKeys = useMemo(() => coords.map((c) => graphKey(c.database, c.graph)), [coords]);
  const namespaceKeysStr = namespaceKeys.join(",");

  useEffect(() => {
    if (namespaceKeys.length === 0) return;
    if (graph !== "all" && !namespaceKeys.includes(graph)) setGraph("all");
    const hasStored = coords.some((c) => c.kind === "stored");
    // Hidden by default: the synthetic /schema graph (when real graphs exist)
    // and the file-candidate layer (un-promoted contributed/scraped files) —
    // toggle either back on from the legend Graphs list.
    setHiddenNamespaces(
      namespaceKeys.filter(
        (k) => (hasStored && k.endsWith("/schema")) || k.includes("file_candidates"),
      ),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [namespaceKeysStr]);

  // ── Active namespaces ────────────────────────────────────────────────────────
  const activeNamespaces = useMemo(() => {
    if (graph !== "all" && namespaceKeys.includes(graph)) return new Set([graph]);
    return new Set(namespaceKeys.filter((n) => !hiddenNamespaces.includes(n)));
  }, [graph, namespaceKeys, hiddenNamespaces]);

  // ── Unfiltered node map (for inspector + breadcrumbs) ────────────────────────
  const nodesByCompositeId = useMemo(
    () => new Map(nodes.map((n) => [keyOf(n), n])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [exp.version],
  );

  // ── Canvas fallback: visible nodes/edges filtered by namespace + label ───────
  // (WebGL path filters inside buildGraphologyGraph via activeNamespaces + hiddenLabels store.)
  const edgeSrcKey = (e: GraphRelationship) => `${e.namespace ?? ""}::${e.source_id}`;
  const edgeDstKey = (e: GraphRelationship) =>
    `${(e.properties?.target_namespace as string | undefined) ?? e.namespace ?? ""}::${e.target_id}`;

  const visNodes = useMemo(
    () =>
      nodes.filter(
        (n) =>
          activeNamespaces.has(n.namespace ?? "") &&
          !hiddenLabels.includes(n.labels[0] ?? ""),
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [exp.version, activeNamespaces, hiddenLabels],
  );
  const visEdges = useMemo(() => {
    const visIds = new Set(visNodes.map(keyOf));
    return edges.filter((e) => visIds.has(edgeSrcKey(e)) && visIds.has(edgeDstKey(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visNodes, exp.version]);

  // ── Deep-link focus ──────────────────────────────────────────────────────────
  const focusedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!focusNodeId || nodes.length === 0) return;
    if (focusedRef.current === focusNodeId) return;
    const match = focusNodeId.includes("::")
      ? nodesByCompositeId.get(focusNodeId)
      : nodes.find((n) => n.id === focusNodeId);
    if (match) {
      focusedRef.current = focusNodeId;
      focusNode(keyOf(match));
    }
    // exp.version (not `nodes`) — the accumulator array is mutated in place, so
    // its reference never changes; version is what signals "new nodes arrived"
    // and lets a deep-link target found in a later chunk still get focused.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focusNodeId, exp.version]);

  // ── onSelectedNodeChange bridge ───────────────────────────────────────────────
  const selectedNode = selectedNodeId ? (nodesByCompositeId.get(selectedNodeId) ?? null) : null;
  useEffect(() => {
    onSelectedNodeChange?.(selectedNode ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedNode]);

  // ── Renderer selection ────────────────────────────────────────────────────────
  // useState so sigma instance availability triggers a chrome re-render.
  const [sigma, setSigma] = useState<Sigma | null>(null);
  const useWebGL = renderer === "webgl" || (renderer !== "canvas" && webglAvailable());

  // graphRef for CommandBar + keyboard walk (sigma graph or null for canvas path).
  const graphRef = useRef<import("graphology").default | null>(null);
  const handleSigmaReady = useCallback((s: Sigma) => {
    setSigma(s);
    graphRef.current = s.getGraph();
  }, []);

  // ── Zoom controls (WebGL path) ────────────────────────────────────────────────
  const zoomIn = useCallback(() => {
    const cam = sigma?.getCamera();
    if (!cam) return;
    cam.animate({ ratio: cam.getState().ratio / 1.5 }, { duration: 200 });
  }, [sigma]);
  const zoomOut = useCallback(() => {
    const cam = sigma?.getCamera();
    if (!cam) return;
    cam.animate({ ratio: cam.getState().ratio * 1.5 }, { duration: 200 });
  }, [sigma]);
  const zoomFit = useCallback(() => {
    sigma?.getCamera().animatedReset();
  }, [sigma]);

  // ── Namespace counts for LegendDock ──────────────────────────────────────────
  const namespaceCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const n of nodes) {
      const ns = n.namespace ?? "";
      counts[ns] = (counts[ns] ?? 0) + 1;
    }
    return counts;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [exp.version]);

  // ── Keyboard walk ─────────────────────────────────────────────────────────────
  useKeyboardWalk(graphRef);

  // ── Galaxy substrate + vignette backgrounds ───────────────────────────────────
  const galaxyBg = isDark
    ? "radial-gradient(120% 100% at 50% -10%, hsl(213 30% 12%), hsl(213 32% 7%) 60%, hsl(214 36% 4%) 100%)"
    : "radial-gradient(120% 100% at 50% -10%, hsl(210 36% 99%), hsl(210 30% 96%) 100%)";

  // ── Render ────────────────────────────────────────────────────────────────────
  // Flex row: canvas area + docked inspector. The inspector docks beside the
  // canvas (which shrinks via SigmaCanvas's ResizeObserver) instead of
  // floating over it, so selecting a node never hides part of the graph.
  return (
    <div className="ky-flex ky-h-full ky-w-full ky-overflow-hidden">
      <div className="ky-relative ky-h-full ky-min-w-0 ky-flex-1 ky-overflow-hidden">
      {/* Galaxy substrate (behind the transparent canvas) */}
      <div className="ky-pointer-events-none ky-absolute ky-inset-0" style={{ background: galaxyBg }} />

      {/* Full loading state */}
      {exp.isLoading && nodes.length === 0 && (
        <div className="ky-absolute ky-inset-0 ky-z-10 ky-flex ky-items-center ky-justify-center ky-text-sm ky-text-muted-foreground">
          Loading graph…
        </div>
      )}

      {/* Full error state (no nodes at all) */}
      {exp.isError && nodes.length === 0 && (
        <div className="ky-absolute ky-inset-0 ky-z-10 ky-flex ky-items-center ky-justify-center ky-text-sm ky-text-destructive">
          Failed to load graph: {(exp.error as Error)?.message}
        </div>
      )}

      {/* Empty state */}
      {!exp.isLoading && !exp.isError && nodes.length === 0 && (
        <div className="ky-absolute ky-inset-0 ky-z-10 ky-flex ky-flex-col ky-items-center ky-justify-center ky-gap-2 ky-text-muted-foreground">
          <Network className="ky-h-8 ky-w-8" />
          <div className="ky-text-sm">
            No graph data yet — ingest some tables or connect a source to see the graph.
          </div>
        </div>
      )}

      {/* Community hulls layer — under the sigma canvases, WebGL only */}
      {useWebGL && (
        <HullsLayer sigma={sigma} nodes={nodes} edges={edges} version={exp.version} />
      )}

      {/* Canvas renderers */}
      {useWebGL ? (
        <SigmaCanvas
          nodes={nodes}
          edges={edges}
          positions={exp.acc.positions}
          version={exp.version}
          activeNamespaces={activeNamespaces}
          onNodeClick={(id) => {
            if (id === "") {
              selectNode(null);
            } else {
              pushTrail(id);
              selectNode(id);
            }
          }}
          onNodeHover={hoverNode}
          onNodeDoubleClick={(id) => {
            // Focus-neighborhood toggle: double-click enters/exits focus mode.
            setFocusMode(focusModeId === id ? null : id);
          }}
          onSigmaReady={handleSigmaReady}
        />
      ) : (
        <GraphCanvas
          nodes={visNodes}
          edges={visEdges}
          layout={layout}
          showEdgeLabels={showEdgeLabels}
          onNodeClick={(id) => {
            if (id === "") {
              selectNode(null);
            } else {
              pushTrail(id);
              selectNode(id);
            }
          }}
          onNodeHover={hoverNode}
          onNodeDoubleClick={(id) => {
            setFocusMode(focusModeId === id ? null : id);
          }}
        />
      )}

      {/* Vignette to draw the eye to the centre */}
      <div
        className="ky-pointer-events-none ky-absolute ky-inset-0"
        style={{
          background: isDark
            ? "radial-gradient(125% 125% at 50% 50%, transparent 55%, rgba(0,0,0,0.38) 100%)"
            : "radial-gradient(125% 125% at 50% 50%, transparent 65%, rgba(15,23,42,0.05) 100%)",
        }}
      />

      {/* Layout computing pill */}
      {exp.layoutComputing && nodes.length > 0 && (
        <div className="ky-absolute ky-left-1/2 ky-top-3 ky-z-10 -ky-translate-x-1/2">
          <div className="ky-glass ky-rounded-full ky-px-3 ky-py-1 ky-text-xs ky-text-muted-foreground">
            Computing layout server-side… {nodes.length.toLocaleString()} nodes loaded
          </div>
        </div>
      )}

      {/* Partial-error retry pill (stream failed but we have some data) */}
      {exp.isError && nodes.length > 0 && (
        <div className="ky-absolute ky-left-1/2 ky-top-3 ky-z-10 -ky-translate-x-1/2">
          <button
            type="button"
            onClick={() => exp.refetch()}
            className="ky-glass ky-flex ky-items-center ky-gap-1.5 ky-rounded-full ky-px-3 ky-py-1 ky-text-xs ky-text-destructive ky-transition-colors hover:ky-text-foreground"
          >
            <RefreshCw className="ky-h-3 ky-w-3" />
            Partial graph — a stream failed. Retry
          </button>
        </div>
      )}

      {/* Chrome layer — only when showChrome is true */}
      {showChrome && (
        <>
          <CommandBar graphRef={graphRef} />
          <BreadcrumbTrail nodesByCompositeId={nodesByCompositeId} />
          <LegendDock
            stats={exp.acc.stats}
            coords={coords}
            namespaceCounts={namespaceCounts}
            visibleNodes={useWebGL ? nodes.length : visNodes.length}
            visibleEdges={useWebGL ? edges.length : visEdges.length}
          />

          {/* Minimap — WebGL only */}
          {useWebGL && <Minimap sigma={sigma} />}

          {/* Zoom controls — WebGL only (canvas fallback has its own CanvasControls) */}
          {useWebGL && (
            <div className="ky-absolute ky-bottom-4 ky-right-4 ky-z-20 ky-glass ky-flex ky-flex-col ky-overflow-hidden ky-rounded-lg ky-border ky-border-border">
              <button
                type="button"
                onClick={zoomIn}
                title="Zoom in"
                aria-label="Zoom in"
                className="ky-flex ky-h-8 ky-w-8 ky-items-center ky-justify-center ky-text-muted-foreground ky-transition-colors hover:ky-bg-accent hover:ky-text-foreground"
              >
                <Plus className="ky-h-4 ky-w-4" />
              </button>
              <button
                type="button"
                onClick={zoomFit}
                title="Fit to view"
                aria-label="Fit to view"
                className="ky-flex ky-h-8 ky-w-8 ky-items-center ky-justify-center ky-border-y ky-border-border/60 ky-text-muted-foreground ky-transition-colors hover:ky-bg-accent hover:ky-text-foreground"
              >
                <Maximize2 className="ky-h-3.5 ky-w-3.5" />
              </button>
              <button
                type="button"
                onClick={zoomOut}
                title="Zoom out"
                aria-label="Zoom out"
                className="ky-flex ky-h-8 ky-w-8 ky-items-center ky-justify-center ky-text-muted-foreground ky-transition-colors hover:ky-bg-accent hover:ky-text-foreground"
              >
                <Minus className="ky-h-4 ky-w-4" />
              </button>
            </div>
          )}
        </>
      )}
      </div>

      {/* Docked inspector — sits beside the canvas, never over it */}
      {showChrome && (
        <InspectorPanel
          node={selectedNode}
          edges={edges}
          nodesByCompositeId={nodesByCompositeId}
        />
      )}
    </div>
  );
}
