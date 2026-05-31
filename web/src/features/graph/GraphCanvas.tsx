import "@xyflow/react/dist/style.css";

import { useEffect, useMemo } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Controls,
  MiniMap,
  Background,
  BackgroundVariant,
  MarkerType,
  useReactFlow,
  type Node,
  type Edge,
} from "@xyflow/react";

import type { GraphNode, GraphRelationship } from "@/sdk/graph";
import type { LayoutAlgorithm } from "@/sdk/graph-layout";
import { computeLayout, getLabelColor, getRelationshipColor } from "@/sdk/graph-layout";
import { GraphNodeView } from "./GraphNodeView";
import { CanvasContext } from "./canvas-context";
import { useGraphStore } from "./graph-store";
import { useTheme } from "@/lib/theme";

const nodeTypes = { graphNode: GraphNodeView };

export interface GraphCanvasProps {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  layout: LayoutAlgorithm;
  showEdgeLabels: boolean;
  showMiniMap: boolean;
  onNodeClick: (id: string) => void;
  onNodeHover: (id: string | null) => void;
}

// Cross-DB node ids collide — composite ids `${namespace}::${id}` keep
// layout, React Flow nodes, and selection unambiguous. GraphView translates
// back to (database, graph, bare-id) when calling the API.
const keyOf = (n: { id: string; namespace?: string }) =>
  `${n.namespace ?? ""}::${n.id}`;
const edgeEndpointKey = (
  e: { source_id: string; target_id: string; namespace?: string },
  end: "source_id" | "target_id",
) => `${e.namespace ?? ""}::${e[end]}`;

function GraphCanvasInner({
  nodes,
  edges,
  layout,
  showEdgeLabels,
  showMiniMap,
  onNodeClick,
  onNodeHover,
}: GraphCanvasProps) {
  const { fitView } = useReactFlow();
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const resolvedTheme = useTheme((s) => s.resolved);
  const isDark = resolvedTheme === "dark";

  const compositeNodes = useMemo(
    () => nodes.map((n) => ({ ...n, id: keyOf(n) })),
    [nodes],
  );
  const compositeEdges = useMemo(
    () =>
      edges.map((e) => ({
        ...e,
        id: `${e.namespace ?? ""}::${e.id}`,
        source_id: edgeEndpointKey(e, "source_id"),
        target_id: edgeEndpointKey(e, "target_id"),
      })),
    [edges],
  );

  // Cheap memo deps — array lengths instead of join'd id strings (the old
  // approach allocated a ~200KB string per render at 4000 nodes).
  const nodeCount = compositeNodes.length;
  const edgeCount = compositeEdges.length;

  const positions = useMemo(
    () => computeLayout(layout, compositeNodes, compositeEdges, 1400, 900),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [nodeCount, edgeCount, layout],
  );

  // Per-node neighbour map + per-rel-type adjacency — handed to each node
  // via context so individual GraphNodeView instances can compute their own
  // dim/highlight without the parent rebuilding the node array on hover.
  const neighborsByCompositeId = useMemo(() => {
    const m = new Map<string, Set<string>>();
    for (const e of compositeEdges) {
      let s = m.get(e.source_id);
      if (!s) m.set(e.source_id, (s = new Set()));
      s.add(e.target_id);
      let t = m.get(e.target_id);
      if (!t) m.set(e.target_id, (t = new Set()));
      t.add(e.source_id);
    }
    return m;
  }, [compositeEdges]);
  const nodesByRelType = useMemo(() => {
    const m = new Map<string, Set<string>>();
    for (const e of compositeEdges) {
      let s = m.get(e.relationship_type);
      if (!s) m.set(e.relationship_type, (s = new Set()));
      s.add(e.source_id);
      s.add(e.target_id);
    }
    return m;
  }, [compositeEdges]);
  const contextValue = useMemo(
    () => ({ neighborsByCompositeId, nodesByRelType }),
    [neighborsByCompositeId, nodesByRelType],
  );

  // Structural-only — never depend on hover/selection here.
  const fnodes: Node[] = useMemo(
    () =>
      compositeNodes.map((n) => ({
        id: n.id,
        type: "graphNode",
        position: positions.get(n.id) ?? { x: 0, y: 0 },
        data: {
          label:
            typeof n.properties.name === "string"
              ? n.properties.name
              : n.id.split("::").pop() ?? n.id,
          labels: n.labels,
          color: getLabelColor(n.labels[0] ?? ""),
        },
      })),
    [compositeNodes, positions],
  );

  // Edges adapt opacity to the theme so they stay visible in both modes
  // (dark mode swallows light colours; light mode swallows dark ones).
  const edgeStrokeOpacity = isDark ? 0.5 : 0.6;
  const fedges: Edge[] = useMemo(() => {
    return compositeEdges.map((r) => {
      const color = getRelationshipColor(r.relationship_type);
      return {
        id: r.id,
        source: r.source_id,
        target: r.target_id,
        type: "default",
        label: showEdgeLabels ? r.relationship_type : undefined,
        labelShowBg: true,
        labelBgStyle: { fill: "hsl(var(--background))", fillOpacity: 0.9 },
        labelBgPadding: [3, 1] as [number, number],
        labelBgBorderRadius: 3,
        labelStyle: {
          fontSize: 8.5,
          fill: color,
          fontWeight: 600,
        },
        markerEnd: {
          type: MarkerType.ArrowClosed,
          color,
          width: 10,
          height: 10,
        },
        style: {
          stroke: color,
          strokeWidth: 1.1,
          strokeOpacity: edgeStrokeOpacity,
        },
      };
    });
  }, [compositeEdges, showEdgeLabels, edgeStrokeOpacity]);

  useEffect(() => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        if (selectedNodeId) {
          fitView({
            nodes: [{ id: selectedNodeId }],
            padding: 0.5,
            maxZoom: 1.6,
            duration: 360,
          });
        } else {
          fitView({ padding: 0.18, maxZoom: 1.3, duration: 240 });
        }
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodeCount, layout, selectedNodeId]);

  // Theme-aware paint surface — ReactFlow needs an explicit colour for its
  // own backdrop, so we resolve it from the CSS variables that already
  // describe the rest of the app.
  const dotColor = isDark ? "hsl(217.2 32.6% 17.5%)" : "hsl(214.3 31.8% 91.4%)";
  const controlsStyle = {
    background: "hsl(var(--card))",
    border: "1px solid hsl(var(--border))",
    borderRadius: 6,
    boxShadow: "0 4px 12px hsl(var(--foreground) / 0.08)",
  };

  return (
    <CanvasContext.Provider value={contextValue}>
      <ReactFlow
        nodes={fnodes}
        edges={fedges}
        nodeTypes={nodeTypes}
        fitView
        minZoom={0.05}
        maxZoom={4}
        proOptions={{ hideAttribution: true }}
        style={{ background: "hsl(var(--background))" }}
        defaultEdgeOptions={{ type: "default" }}
        onNodeClick={(_, n) => onNodeClick(n.id)}
        onNodeMouseEnter={(_, n) => onNodeHover(n.id)}
        onNodeMouseLeave={() => onNodeHover(null)}
        onPaneClick={() => onNodeClick("")}
      >
        <Controls showInteractive={false} style={controlsStyle} />
        {showMiniMap && (
          <MiniMap
            pannable
            zoomable
            nodeColor={(n) => (n.data as { color?: string }).color ?? "#94a3b8"}
            maskColor={isDark ? "rgba(0,0,0,0.55)" : "rgba(15,23,42,0.08)"}
            style={controlsStyle}
          />
        )}
        <Background
          variant={BackgroundVariant.Dots}
          gap={28}
          size={1}
          color={dotColor}
        />
      </ReactFlow>
    </CanvasContext.Provider>
  );
}

export function GraphCanvas(props: GraphCanvasProps) {
  return (
    <ReactFlowProvider>
      <GraphCanvasInner {...props} />
    </ReactFlowProvider>
  );
}
