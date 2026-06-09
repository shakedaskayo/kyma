import { useCallback, useEffect, useMemo, useState } from "react";
import { Network } from "lucide-react";
import { useGraphStore } from "./graph-store";
import {
  useUnifiedGraph,
  useAllDatabaseGraphs,
  graphKey,
  type GraphCoord,
} from "./useGraph";
import {
  expandNeighbors,
  getNode,
  type GraphNode,
  type GraphRelationship,
} from "@/sdk/graph";
import { useSession } from "@/sdk/session";
import { useTheme } from "@/lib/theme";
import { GraphCanvas } from "./GraphCanvas";
import { GraphSidebar } from "./GraphSidebar";

/**
 * Cross-database unified graph view. Always merges every (database, graph)
 * pair into one canvas. Rendered on a WebGL/canvas force renderer with a dark
 * "galaxy" treatment: community hulls, glowing centrality-sized nodes, curved
 * relationship-family edges, and animated flow on the focused neighbourhood.
 */

/** Above this many visible nodes, overview mode shows only landmarks + the
 * expanded/selected neighbourhoods instead of the full hairball. */
const OVERVIEW_LIMIT = 280;
const LANDMARK_COUNT = 60;

const keyOf = (n: { id: string; namespace?: string }) => `${n.namespace ?? ""}::${n.id}`;
const edgeSrcKey = (e: GraphRelationship) => `${e.namespace ?? ""}::${e.source_id}`;
const edgeDstKey = (e: GraphRelationship) =>
  `${(e.properties?.target_namespace as string | undefined) ?? e.namespace ?? ""}::${e.target_id}`;

export function GraphView() {
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

  const { endpoint, token } = useSession();
  const isDark = useTheme((s) => s.resolved === "dark");
  const allGraphs = useAllDatabaseGraphs();

  const coords: GraphCoord[] = useMemo(() => allGraphs.data ?? [], [allGraphs.data]);
  const namespaceKeys = useMemo(() => coords.map(graphKey), [coords]);
  const namespaceKeysStr = namespaceKeys.join(",");
  const unified = useUnifiedGraph(coords, 800);

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
    if (unified.data) {
      setNodes(unified.data.nodes);
      setEdges(unified.data.edges);
    }
  }, [unified.data]);

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
      const node = nodesByCompositeId.get(compositeId);
      setExpandedIds((prev) => new Set(prev).add(compositeId));
      if (!node || !node.namespace || !node.database) return;
      const ns = node.namespace;
      const db = node.database;
      const graphName = ns.slice(db.length + 1);
      if (!graphName) return;
      const exp = await expandNeighbors({
        endpoint,
        token,
        database: db,
        graph: graphName,
        nodeIds: [node.id],
        direction: "both",
      });
      setEdges((prev) => {
        const seen = new Set(prev.map((e) => `${e.namespace ?? ""}::${e.id}`));
        return [
          ...prev,
          ...exp.edges
            .filter((e) => !seen.has(`${ns}::${e.id}`))
            .map((e) => ({ ...e, namespace: ns, database: db })),
        ];
      });
      const have = new Set(nodes.filter((n) => n.namespace === ns).map((n) => n.id));
      const missing = exp.new_node_ids.filter((nid) => !have.has(nid));
      if (missing.length) {
        const fetched = await Promise.all(
          missing.map((nid) =>
            getNode({ endpoint, token, database: db, graph: graphName, id: nid }).catch(() => null),
          ),
        );
        const add = fetched
          .filter((n): n is GraphNode => n != null)
          .map((n) => ({ ...n, namespace: ns, database: db }));
        if (add.length) {
          setNodes((prev) => {
            const seen = new Set(prev.map((n) => keyOf(n)));
            return [...prev, ...add.filter((n) => !seen.has(keyOf(n)))];
          });
        }
      }
    },
    [endpoint, token, nodes, nodesByCompositeId],
  );

  const selectedNode = selectedNodeId ? nodesByCompositeId.get(selectedNodeId) ?? null : null;

  const loadingText =
    unified.progress.total > 0 && unified.progress.settled < unified.progress.total
      ? `Loading ${unified.progress.settled}/${unified.progress.total}`
      : null;

  // Dark "galaxy" substrate vs. clean light surface.
  const galaxyBg = isDark
    ? "radial-gradient(120% 100% at 50% -10%, hsl(213 30% 12%), hsl(213 32% 7%) 60%, hsl(214 36% 4%) 100%)"
    : "radial-gradient(120% 100% at 50% -10%, hsl(210 36% 99%), hsl(210 30% 96%) 100%)";

  return (
    <div className="flex h-full w-full bg-background">
      <div className="relative flex-1 overflow-hidden">
        {/* Galaxy substrate (behind the transparent canvas) */}
        <div className="pointer-events-none absolute inset-0" style={{ background: galaxyBg }} />

        {unified.isLoading && !unified.progress.hasAny && (
          <div className="absolute inset-0 z-10 flex items-center justify-center text-sm text-muted-foreground">
            Loading graph…
          </div>
        )}
        {unified.isError && (
          <div className="absolute inset-0 z-10 flex items-center justify-center text-sm text-destructive">
            Failed to load graph: {(unified.error as Error)?.message}
          </div>
        )}
        {!unified.isLoading && !unified.isError && nodes.length === 0 && (
          <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 text-muted-foreground">
            <Network className="h-8 w-8" />
            <div className="text-sm">
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
          className="pointer-events-none absolute inset-0"
          style={{
            background: isDark
              ? "radial-gradient(125% 125% at 50% 50%, transparent 55%, rgba(0,0,0,0.38) 100%)"
              : "radial-gradient(125% 125% at 50% 50%, transparent 65%, rgba(15,23,42,0.05) 100%)",
          }}
        />

        {/* Overview hint */}
        {overviewActive && (
          <div className="absolute left-1/2 top-3 z-10 -translate-x-1/2">
            <button
              type="button"
              onClick={() => setOverview(false)}
              className="glass rounded-full px-3 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
              title="Show every node (may be dense)"
            >
              Overview · showing {visNodes.length.toLocaleString()} of{" "}
              {baseNodes.length.toLocaleString()} — show all
            </button>
          </div>
        )}
      </div>

      <GraphSidebar
        coords={coords}
        namespaceCounts={unified.data?.namespaceCounts}
        stats={unified.data?.stats}
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
    </div>
  );
}
