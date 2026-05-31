import { useEffect, useMemo, useState } from "react";
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
import { GraphCanvas } from "./GraphCanvas";
import { GraphSidebar } from "./GraphSidebar";

/**
 * Cross-database unified graph view. Always merges every (database, graph)
 * pair into one canvas — the global database switcher does not constrain
 * this view. UI is intentionally distinct from the rest of the app: dark
 * canvas + 320px right sidebar, modeled on safishamsi/graphify.
 */
export function GraphView() {
  const graph = useGraphStore((s) => s.graph);
  const setGraph = useGraphStore((s) => s.setGraph);
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const selectNode = useGraphStore((s) => s.selectNode);
  const hoverNode = useGraphStore((s) => s.hoverNode);
  const labelFilter = useGraphStore((s) => s.labelFilter);
  const hiddenLabels = useGraphStore((s) => s.hiddenLabels);
  const hiddenNamespaces = useGraphStore((s) => s.hiddenNamespaces);
  const setHiddenNamespaces = useGraphStore((s) => s.setHiddenNamespaces);
  const showEdgeLabels = useGraphStore((s) => s.showEdgeLabels);
  const showMiniMap = useGraphStore((s) => s.showMiniMap);
  const setShowMiniMap = useGraphStore((s) => s.setShowMiniMap);
  const layout = useGraphStore((s) => s.layout);

  const { endpoint, token } = useSession();
  const allGraphs = useAllDatabaseGraphs();

  const coords: GraphCoord[] = useMemo(
    () => allGraphs.data ?? [],
    [allGraphs.data],
  );
  const namespaceKeys = useMemo(() => coords.map(graphKey), [coords]);
  const namespaceKeysStr = namespaceKeys.join(",");
  // Lower default cap per namespace — the cross-DB unified view is wide
  // (many namespaces), so per-namespace fetches should be lean. Drill-down
  // via the sidebar focus picker still gets the full graph.
  const unified = useUnifiedGraph(coords, 800);

  // When the set of graphs changes (new connector, new DB, first load):
  // recover a stale focus selection, and default to hiding `schema`
  // namespaces when real stored graphs exist (schemas are table-level noise
  // when there's ingested content). Also disable the minimap if the unified
  // payload is huge — it becomes a soup of dots otherwise.
  useEffect(() => {
    if (namespaceKeys.length === 0) return;
    if (graph !== "all" && !namespaceKeys.includes(graph)) setGraph("all");
    const hasStored = coords.some((c) => c.kind === "stored");
    setHiddenNamespaces(
      hasStored ? namespaceKeys.filter((k) => k.endsWith("/schema")) : [],
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [namespaceKeysStr]);

  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [edges, setEdges] = useState<GraphRelationship[]>([]);

  // Seed local state from the streaming hook — `unified.data` updates as
  // each namespace lands, so the canvas grows progressively.
  useEffect(() => {
    if (unified.data) {
      setNodes(unified.data.nodes);
      setEdges(unified.data.edges);
    }
  }, [unified.data]);

  // Auto-disable the minimap on huge unified graphs — it stops being a
  // useful overview and starts being a noisy soup.
  useEffect(() => {
    if (nodes.length > 800 && showMiniMap) setShowMiniMap(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodes.length > 800]);

  // Composite id helper — keep canvas/selection/lookup uniformly namespaced
  // so id collisions across databases don't clash.
  const keyOf = (n: { id: string; namespace?: string }) =>
    `${n.namespace ?? ""}::${n.id}`;

  const nodesByCompositeId = useMemo(
    () => new Map(nodes.map((n) => [keyOf(n), n])),
    [nodes],
  );

  const activeNamespaces = useMemo(() => {
    if (graph !== "all" && namespaceKeys.includes(graph)) return new Set([graph]);
    return new Set(namespaceKeys.filter((n) => !hiddenNamespaces.includes(n)));
  }, [graph, namespaceKeys, hiddenNamespaces]);

  // Apply namespace + label visibility before handing to the canvas. Label-
  // filter (single-type focus) is handled inside the node component via
  // dimming, so we don't filter it out here.
  void labelFilter;
  const visNodes = useMemo(
    () =>
      nodes.filter(
        (n) =>
          activeNamespaces.has(n.namespace ?? "") &&
          !hiddenLabels.includes(n.labels[0] ?? ""),
      ),
    [nodes, activeNamespaces, hiddenLabels],
  );
  const visEdges = useMemo(() => {
    const visIds = new Set(visNodes.map(keyOf));
    return edges.filter(
      (e) =>
        visIds.has(`${e.namespace ?? ""}::${e.source_id}`) &&
        visIds.has(`${e.namespace ?? ""}::${e.target_id}`),
    );
  }, [edges, visNodes]);

  async function handleExpand() {
    const node = selectedNodeId ? nodesByCompositeId.get(selectedNodeId) : null;
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
    const have = new Set(
      nodes.filter((n) => n.namespace === ns).map((n) => n.id),
    );
    const missing = exp.new_node_ids.filter((nid) => !have.has(nid));
    if (missing.length) {
      const fetched = await Promise.all(
        missing.map((nid) =>
          getNode({ endpoint, token, database: db, graph: graphName, id: nid }).catch(
            () => null,
          ),
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
  }

  const selectedNode = selectedNodeId
    ? nodesByCompositeId.get(selectedNodeId) ?? null
    : null;

  // Friendly progress text while per-namespace fetches stream in. Once all
  // settle, this hides itself.
  const loadingText =
    unified.progress.total > 0 && unified.progress.settled < unified.progress.total
      ? `Loading ${unified.progress.settled}/${unified.progress.total}`
      : null;

  return (
    <div className="flex h-full w-full bg-background">
      <div className="relative flex-1">
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
              No graph data yet — ingest some tables or connect a source to see
              the graph.
            </div>
          </div>
        )}
        <GraphCanvas
          nodes={visNodes}
          edges={visEdges}
          layout={layout}
          showEdgeLabels={showEdgeLabels}
          showMiniMap={showMiniMap}
          onNodeClick={(id) => selectNode(id === "" ? null : id)}
          onNodeHover={hoverNode}
        />
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
        onExpand={handleExpand}
      />
    </div>
  );
}
