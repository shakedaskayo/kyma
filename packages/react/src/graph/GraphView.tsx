import { useCallback, useEffect, useMemo, useState } from "react";
import { Network } from "lucide-react";
import { useGraphStore } from "./graph-store";
import { type GraphCoord, graphKey, useKymaGraph } from "../hooks/useKymaGraph";
import type { GraphNode, GraphRelationship } from "@kyma-ai/client";
import { useKymaContext } from "../provider/context";
import { GraphCanvas } from "./GraphCanvas";
import { GraphSidebar } from "./GraphSidebar";

/**
 * Cross-database unified graph view. Always merges every (database, graph)
 * pair into one canvas. Rendered on a WebGL/canvas force renderer with a dark
 * "galaxy" treatment: community hulls, glowing centrality-sized nodes, curved
 * relationship-family edges, and animated flow on the focused neighbourhood.
 *
 * NOTE: This component must be rendered inside a GraphStoreContext provider
 * (supplied by KymaGraph). It consumes useKymaGraph internally — callers
 * control graph discovery via the hook args passed through the parent wrapper.
 */

/** Above this many visible nodes, overview mode shows only landmarks + the
 * expanded/selected neighbourhoods instead of the full hairball. */
const OVERVIEW_LIMIT = 280;
const LANDMARK_COUNT = 60;

const keyOf = (n: { id: string; namespace?: string }) => `${n.namespace ?? ""}::${n.id}`;
const edgeSrcKey = (e: GraphRelationship) => `${e.namespace ?? ""}::${e.source_id}`;
const edgeDstKey = (e: GraphRelationship) =>
  `${(e.properties?.target_namespace as string | undefined) ?? e.namespace ?? ""}::${e.target_id}`;

export interface GraphViewProps {
  /** Graphs to merge; omit to load all graphs of the default database. */
  graphs?: Array<{ database?: string; graph: string }>;
  /**
   * "all-databases": discover every (database, graph) across the deployment.
   * When set, `graphs` is ignored.
   */
  discover?: "all-databases";
  realm?: string;
  limit?: number;
  /** When false, the sidebar (legend, controls, inspector) is hidden. Default true. */
  showSidebar?: boolean;
  /**
   * Fires with the RESOLVED selected node whenever the selection changes
   * (null on deselect). This is the public-callback bridge for KymaGraph —
   * the full GraphNode lives in this component's node map, not in the store.
   */
  onSelectedNodeChange?: (node: GraphNode | null) => void;
}

export function GraphView({
  graphs,
  discover,
  realm,
  limit,
  showSidebar = true,
  onSelectedNodeChange,
}: GraphViewProps) {
  const graph = useGraphStore((s) => s.graph);
  const setGraph = useGraphStore((s) => s.setGraph);
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const selectNode = useGraphStore((s) => s.selectNode);
  const hoverNode = useGraphStore((s) => s.hoverNode);
  const hiddenLabels = useGraphStore((s) => s.hiddenLabels);
  const hiddenNamespaces = useGraphStore((s) => s.hiddenNamespaces);
  const setHiddenNamespaces = useGraphStore((s) => s.setHiddenNamespaces);
  const showEdgeLabels = useGraphStore((s) => s.showEdgeLabels);
  const layout = useGraphStore((s) => s.layout);
  const overview = useGraphStore((s) => s.overview);
  const setOverview = useGraphStore((s) => s.setOverview);

  const { isDark } = useKymaContext();

  const unified = useKymaGraph({ graphs, discover, realm, limit });

  const coords: GraphCoord[] = unified.coords;
  const namespaceKeys = useMemo(() => coords.map((c) => graphKey(c.database, c.graph)), [coords]);
  const namespaceKeysStr = namespaceKeys.join(",");

  useEffect(() => {
    if (namespaceKeys.length === 0) return;
    if (graph !== "all" && !namespaceKeys.includes(graph)) setGraph("all");
    const hasStored = coords.some((c) => c.kind === "stored");
    // Hidden by default: the synthetic /schema graph (when real graphs exist)
    // and the file-candidate layer (un-promoted contributed/scraped files) —
    // toggle either back on from the Graphs list in the sidebar.
    setHiddenNamespaces(
      namespaceKeys.filter(
        (k) => (hasStored && k.endsWith("/schema")) || k.includes("file_candidates"),
      ),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [namespaceKeysStr]);

  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [edges, setEdges] = useState<GraphRelationship[]>([]);
  // Composite ids the user has drilled into — their neighbourhoods stay visible
  // in overview mode.
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (unified.nodes.length > 0 || unified.edges.length > 0) {
      setNodes(unified.nodes);
      setEdges(unified.edges);
    }
  }, [unified.nodes, unified.edges]);

  const nodesByCompositeId = useMemo(
    () => new Map(nodes.map((n) => [keyOf(n), n])),
    [nodes],
  );

  const activeNamespaces = useMemo(() => {
    if (graph !== "all" && namespaceKeys.includes(graph)) return new Set([graph]);
    return new Set(namespaceKeys.filter((n) => !hiddenNamespaces.includes(n)));
  }, [graph, namespaceKeys, hiddenNamespaces]);

  // Namespace + label visibility.
  const baseNodes = useMemo(
    () =>
      nodes.filter(
        (n) =>
          activeNamespaces.has(n.namespace ?? "") &&
          !hiddenLabels.includes(n.labels[0] ?? ""),
      ),
    [nodes, activeNamespaces, hiddenLabels],
  );
  const baseEdges = useMemo(() => {
    const visIds = new Set(baseNodes.map(keyOf));
    return edges.filter((e) => visIds.has(edgeSrcKey(e)) && visIds.has(edgeDstKey(e)));
  }, [edges, baseNodes]);

  // Overview reduction: keep top-degree "landmarks" + the focused/expanded
  // neighbourhoods so large unified graphs open as a readable map, not a soup.
  const overviewActive = overview && baseNodes.length > OVERVIEW_LIMIT;
  const { visNodes, visEdges } = useMemo(() => {
    if (!overviewActive) return { visNodes: baseNodes, visEdges: baseEdges };
    const deg = new Map<string, number>();
    const adj = new Map<string, Set<string>>();
    for (const e of baseEdges) {
      const s = edgeSrcKey(e);
      const t = edgeDstKey(e);
      deg.set(s, (deg.get(s) ?? 0) + 1);
      deg.set(t, (deg.get(t) ?? 0) + 1);
      (adj.get(s) ?? adj.set(s, new Set()).get(s)!).add(t);
      (adj.get(t) ?? adj.set(t, new Set()).get(t)!).add(s);
    }
    const keep = new Set<string>(
      [...deg.entries()].sort((a, b) => b[1] - a[1]).slice(0, LANDMARK_COUNT).map(([id]) => id),
    );
    const focus = [...expandedIds, selectedNodeId].filter(Boolean) as string[];
    for (const id of focus) {
      keep.add(id);
      for (const nb of adj.get(id) ?? []) keep.add(nb);
    }
    const vn = baseNodes.filter((n) => keep.has(keyOf(n)));
    const ve = baseEdges.filter((e) => keep.has(edgeSrcKey(e)) && keep.has(edgeDstKey(e)));
    return { visNodes: vn, visEdges: ve };
  }, [overviewActive, baseNodes, baseEdges, expandedIds, selectedNodeId]);

  const expandNode = useCallback(
    async (compositeId: string) => {
      setExpandedIds((prev) => new Set(prev).add(compositeId));
      await unified.expandNode(compositeId);
    },
    [unified],
  );

  const selectedNode = selectedNodeId ? nodesByCompositeId.get(selectedNodeId) ?? null : null;

  // Public-callback bridge: surface the resolved node (not just its id).
  useEffect(() => {
    onSelectedNodeChange?.(selectedNode ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedNode]);

  const loadingText =
    unified.progress.total > 0 && unified.progress.settled < unified.progress.total
      ? `Loading ${unified.progress.settled}/${unified.progress.total}`
      : null;

  // Dark "galaxy" substrate vs. clean light surface.
  const galaxyBg = isDark
    ? "radial-gradient(120% 100% at 50% -10%, hsl(213 30% 12%), hsl(213 32% 7%) 60%, hsl(214 36% 4%) 100%)"
    : "radial-gradient(120% 100% at 50% -10%, hsl(210 36% 99%), hsl(210 30% 96%) 100%)";

  return (
    <div className="ky-flex ky-h-full ky-w-full ky-bg-background">
      <div className="ky-relative ky-flex-1 ky-overflow-hidden">
        {/* Galaxy substrate (behind the transparent canvas) */}
        <div className="ky-pointer-events-none ky-absolute ky-inset-0" style={{ background: galaxyBg }} />

        {unified.isLoading && !unified.progress.hasAny && (
          <div className="ky-absolute ky-inset-0 ky-z-10 ky-flex ky-items-center ky-justify-center ky-text-sm ky-text-muted-foreground">
            Loading graph…
          </div>
        )}
        {unified.isError && (
          <div className="ky-absolute ky-inset-0 ky-z-10 ky-flex ky-items-center ky-justify-center ky-text-sm ky-text-destructive">
            Failed to load graph: {(unified.error as Error)?.message}
          </div>
        )}
        {!unified.isLoading && !unified.isError && nodes.length === 0 && (
          <div className="ky-absolute ky-inset-0 ky-z-10 ky-flex ky-flex-col ky-items-center ky-justify-center ky-gap-2 ky-text-muted-foreground">
            <Network className="ky-h-8 ky-w-8" />
            <div className="ky-text-sm">
              No graph data yet — ingest some tables or connect a source to see the graph.
            </div>
          </div>
        )}

        <GraphCanvas
          nodes={visNodes}
          edges={visEdges}
          layout={layout}
          showEdgeLabels={showEdgeLabels}
          onNodeClick={(id) => {
            if (id === "") {
              selectNode(null);
            } else {
              selectNode(id);
              setExpandedIds((prev) => new Set(prev).add(id));
            }
          }}
          onNodeHover={hoverNode}
          onNodeDoubleClick={(id) => void expandNode(id)}
        />

        {/* Vignette to draw the eye to the centre */}
        <div
          className="ky-pointer-events-none ky-absolute ky-inset-0"
          style={{
            background: isDark
              ? "radial-gradient(125% 125% at 50% 50%, transparent 55%, rgba(0,0,0,0.38) 100%)"
              : "radial-gradient(125% 125% at 50% 50%, transparent 65%, rgba(15,23,42,0.05) 100%)",
          }}
        />

        {/* Overview hint */}
        {overviewActive && (
          <div className="ky-absolute ky-left-1/2 ky-top-3 ky-z-10 -ky-translate-x-1/2">
            <button
              type="button"
              onClick={() => setOverview(false)}
              className="ky-glass ky-rounded-full ky-px-3 ky-py-1 ky-text-xs ky-text-muted-foreground ky-transition-colors hover:ky-text-foreground"
              title="Show every node (may be dense)"
            >
              Overview · showing {visNodes.length.toLocaleString()} of{" "}
              {baseNodes.length.toLocaleString()} — show all
            </button>
          </div>
        )}
      </div>

      {showSidebar && (
        <GraphSidebar
          coords={coords}
          namespaceCounts={unified.namespaceCounts}
          stats={unified.stats ?? undefined}
          visibleNodes={visNodes.length}
          visibleEdges={visEdges.length}
          totalNodes={nodes.length}
          totalEdges={edges.length}
          loadingText={loadingText}
          selectedNode={selectedNode}
          nodesByCompositeId={nodesByCompositeId}
          edges={edges}
          onSelectComposite={selectNode}
          onExpand={() => selectedNodeId && void expandNode(selectedNodeId)}
        />
      )}
    </div>
  );
}
