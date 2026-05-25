import { useEffect, useMemo, useState } from "react";
import { Network } from "lucide-react";
import { useGraphStore } from "./graph-store";
import { useGraphOverview, useExpandNeighbors } from "./useGraph";
import { getNode, type GraphNode, type GraphRelationship } from "@/sdk/graph";
import { useSession } from "@/sdk/session";
import { GraphCanvas } from "./GraphCanvas";
import { NodeDetailPanel } from "./NodeDetailPanel";
import { GraphLegend } from "./GraphLegend";
import { CanvasToolbar } from "./CanvasToolbar";

export function GraphView() {
  const graph = useGraphStore((s) => s.graph);
  const layout = useGraphStore((s) => s.layout);
  const setLayout = useGraphStore((s) => s.setLayout);
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const selectNode = useGraphStore((s) => s.selectNode);
  const hoveredNodeId = useGraphStore((s) => s.hoveredNodeId);
  const hoverNode = useGraphStore((s) => s.hoverNode);
  const labelFilter = useGraphStore((s) => s.labelFilter);
  const setLabelFilter = useGraphStore((s) => s.setLabelFilter);

  const overview = useGraphOverview(graph);
  const expand = useExpandNeighbors(graph);
  const { endpoint, token } = useSession();

  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [edges, setEdges] = useState<GraphRelationship[]>([]);

  useEffect(() => {
    if (overview.data) {
      setNodes(overview.data.nodes);
      setEdges(overview.data.edges);
    }
  }, [overview.data]);

  const nodesById = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);

  async function handleExpand(id: string) {
    const exp = await expand([id], "both");
    setEdges((prev) => {
      const seen = new Set(prev.map((e) => e.id));
      return [...prev, ...exp.edges.filter((e) => !seen.has(e.id))];
    });
    const have = new Set(nodes.map((n) => n.id));
    const missing = exp.new_node_ids.filter((nid) => !have.has(nid));
    if (missing.length) {
      const fetched = await Promise.all(
        missing.map((nid) => getNode({ endpoint, token, graph, id: nid }).catch(() => null)),
      );
      const add = fetched.filter((n): n is GraphNode => n != null);
      if (add.length) {
        setNodes((prev) => {
          const seen = new Set(prev.map((n) => n.id));
          return [...prev, ...add.filter((n) => !seen.has(n.id))];
        });
      }
    }
  }

  const selectedNode = selectedNodeId ? nodesById.get(selectedNodeId) ?? null : null;

  return (
    <div className="relative flex h-full w-full">
      <div className="relative flex-1">
        {overview.isLoading && (
          <div className="absolute inset-0 z-10 flex items-center justify-center text-sm text-muted-foreground">
            Loading graph…
          </div>
        )}
        {overview.isError && (
          <div className="absolute inset-0 z-10 flex items-center justify-center text-sm text-destructive">
            Failed to load graph: {(overview.error as Error)?.message}
          </div>
        )}
        {!overview.isLoading && !overview.isError && nodes.length === 0 && (
          <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 text-muted-foreground">
            <Network className="h-8 w-8" />
            <div className="text-sm">No graph data yet — ingest some tables to see the schema graph.</div>
          </div>
        )}
        <GraphCanvas
          nodes={nodes}
          edges={edges}
          layout={layout}
          selectedNodeId={selectedNodeId}
          hoveredNodeId={hoveredNodeId}
          labelFilter={labelFilter}
          onNodeClick={(id) => selectNode(id === "" ? null : id)}
          onNodeHover={hoverNode}
        />
        <div className="pointer-events-none absolute right-3 top-3 z-20">
          <GraphLegend
            stats={overview.data?.stats}
            activeLabel={labelFilter}
            onLabelClick={(l) => setLabelFilter(l === labelFilter ? null : l)}
          />
        </div>
        <div className="pointer-events-none absolute bottom-3 left-1/2 z-20 -translate-x-1/2">
          <CanvasToolbar layout={layout} onLayoutChange={setLayout} />
        </div>
      </div>
      {selectedNode && (
        <NodeDetailPanel
          node={selectedNode}
          edges={edges}
          nodesById={nodesById}
          onClose={() => selectNode(null)}
          onExpand={handleExpand}
          onSelect={(id) => selectNode(id)}
        />
      )}
    </div>
  );
}
