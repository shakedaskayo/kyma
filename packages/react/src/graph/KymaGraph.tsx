/**
 * KymaGraph — public embeddable graph component.
 *
 * Wraps GraphView in an instance-isolated zustand store (createGraphStore via
 * useRef) and a KymaErrorBoundary. Each mounted KymaGraph owns its own store so
 * two instances on the same page never share selection / layout state.
 *
 * Props deliberately omitted (not wired — no dead props shipped):
 *   - renderNodeTooltip: no tooltip escape hatch today. Omitted.
 *   - database: graph targets carry their database per entry in `graphs`;
 *     the provider default covers bare entries. A top-level override would be
 *     ambiguous, so it is omitted.
 *
 * toolbar / sidebar props: Both control the chrome (CommandBar, inspector,
 *   legend, breadcrumbs, minimap). Setting either to false hides the full
 *   chrome surface. They resolve to showChrome on GraphView.
 *
 * renderer prop: "webgl" (Sigma.js) or "canvas" (ForceGraph2D fallback).
 *   Default: auto-detected via webglAvailable() — tries webgl2 then webgl.
 */
import { useCallback, useEffect, useRef, type JSX } from "react";
import type { GraphNode } from "@kyma-ai/client";
import { KymaErrorBoundary } from "../internal/KymaErrorBoundary";
import { GraphStoreContext, createGraphStore } from "./graph-store";
import type { GraphStore } from "./graph-store";
import { GraphView } from "./GraphView";
import type { LayoutAlgorithm } from "@kyma-ai/client";
// Re-export so embedders can query the renderer without rendering.
export { webglAvailable } from "./GraphView";

// ── Public prop surface ────────────────────────────────────────────────────────

export interface KymaGraphProps {
  /** Graphs to render; omit for the default database's graphs. */
  graphs?: Array<{ database?: string; graph: string }>;
  /** "all-databases" discovers every graph across the deployment. */
  discover?: "all-databases";
  /** Height of the container div. Default "100%". */
  height?: number | string;
  /** Initial store layout algorithm. Default "force". */
  layout?: LayoutAlgorithm;
  /**
   * Show chrome (CommandBar, inspector, legend, breadcrumbs, minimap). Default true.
   * Note: toolbar and sidebar are legacy aliases — setting either to false hides
   * the whole chrome surface.
   */
  chrome?: boolean;
  toolbar?: boolean;
  sidebar?: boolean;
  /**
   * Renderer to use. "webgl" uses Sigma.js, "canvas" uses the ForceGraph2D fallback.
   * Default: auto-detected via webglAvailable().
   */
  renderer?: "webgl" | "canvas";
  /** Initial search/focus query seeded into the store on first render. */
  focusQuery?: string;
  /** Deep-link target node id (raw or composite `ns::id`) to center + highlight
   *  once it loads. Changing it re-focuses. */
  focusNodeId?: string;
  /** Override the realm passed to the graph data layer. */
  realm?: string;
  /** Extra CSS class for the container div. */
  className?: string;
  /** Inline style for the container div. */
  style?: React.CSSProperties;
  /** Custom fallback node shown by the error boundary when a render error occurs. */
  fallback?: React.ReactNode;
  /**
   * Called when a node is clicked. Wired via store subscription: GraphView
   * writes selectedNodeId to the store on click; KymaGraph subscribes to that
   * field and calls onNodeClick with the resolved GraphNode.
   */
  onNodeClick?: (node: GraphNode) => void;
  /**
   * Called whenever the selection changes (0 or 1 node in the current design).
   * Wired the same way as onNodeClick via store subscription.
   */
  onSelectionChange?: (nodes: GraphNode[]) => void;
}

// ── Component ─────────────────────────────────────────────────────────────────

/**
 * Public embeddable graph component with per-instance isolated state.
 *
 * Must be rendered inside a KymaProvider.
 */
export function KymaGraph(props: KymaGraphProps): JSX.Element {
  const {
    graphs,
    discover,
    height = "100%",
    layout = "force",
    chrome,
    toolbar = true,
    sidebar = true,
    renderer,
    focusQuery,
    focusNodeId,
    realm,
    className,
    style,
    fallback,
    onNodeClick,
    onSelectionChange,
  } = props;

  // One store per mounted instance — never recreated on re-render.
  const storeRef = useRef<GraphStore | null>(null);
  if (!storeRef.current) {
    storeRef.current = createGraphStore({ layout });
  }
  const store = storeRef.current;

  // Seed the initial search query once, after mount.
  useEffect(() => {
    if (focusQuery) {
      store.getState().setSearchQuery(focusQuery);
    }
    // Intentionally only runs once at mount — focusQuery is an initial seed, not a controlled value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Selection callbacks: GraphView resolves the full GraphNode from its node
  // map and reports it via onSelectedNodeChange — no synthetic nodes.
  const handleSelectedNodeChange = useCallback(
    (node: GraphNode | null) => {
      if (node) {
        onNodeClick?.(node);
        onSelectionChange?.([node]);
      } else {
        onSelectionChange?.([]);
      }
    },
    [onNodeClick, onSelectionChange],
  );

  // chrome prop overrides the legacy toolbar/sidebar combo.
  const showChrome = (chrome ?? true) && sidebar && toolbar;

  return (
    <KymaErrorBoundary fallback={fallback}>
      <GraphStoreContext.Provider value={store}>
        <div
          className={className}
          style={{ height, width: "100%", position: "relative", ...style }}
        >
          <GraphView
            graphs={graphs}
            discover={discover}
            realm={realm}
            renderer={renderer}
            showChrome={showChrome}
            focusNodeId={focusNodeId}
            onSelectedNodeChange={
              onNodeClick || onSelectionChange ? handleSelectedNodeChange : undefined
            }
          />
        </div>
      </GraphStoreContext.Provider>
    </KymaErrorBoundary>
  );
}
