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

// Stable reference — must live at module scope so identity never changes
const nodeTypes = { graphNode: GraphNodeView };

export interface GraphCanvasProps {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  layout: LayoutAlgorithm;
  selectedNodeId: string | null;
  hoveredNodeId: string | null;
  labelFilter: string | null;
  relTypeFilter: string | null;
  showEdgeLabels: boolean;
  onNodeClick: (id: string) => void; // pass "" on pane click (deselect)
  onNodeHover: (id: string | null) => void;
}

// ─── Inner component (needs useReactFlow, so must be inside Provider) ────────

function GraphCanvasInner({
  nodes,
  edges,
  layout,
  selectedNodeId,
  hoveredNodeId,
  labelFilter,
  relTypeFilter,
  showEdgeLabels,
  onNodeClick,
  onNodeHover,
}: GraphCanvasProps) {
  const { fitView } = useReactFlow();

  // Stable signature keys for memoisation
  const nodeIdsKey = useMemo(() => nodes.map((n) => n.id).join(","), [nodes]);
  const edgeIdsKey = useMemo(() => edges.map((e) => e.id).join(","), [edges]);

  // 1) Layout — recompute only when node/edge sets or algorithm change
  const positions = useMemo(
    () => computeLayout(layout, nodes, edges, 1400, 900),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [nodeIdsKey, edgeIdsKey, layout],
  );

  // 2) Highlight set — ids of selected/hovered node + all direct neighbours
  const highlight: Set<string> | null = useMemo(() => {
    const focusId = selectedNodeId ?? hoveredNodeId;
    if (!focusId) return null;
    const set = new Set<string>([focusId]);
    for (const e of edges) {
      if (e.source_id === focusId) set.add(e.target_id);
      if (e.target_id === focusId) set.add(e.source_id);
    }
    return set;
  }, [selectedNodeId, hoveredNodeId, edges]);

  // Nodes touched by the active relationship-type filter.
  const relNodes: Set<string> | null = useMemo(() => {
    if (!relTypeFilter) return null;
    const s = new Set<string>();
    for (const e of edges) {
      if (e.relationship_type === relTypeFilter) {
        s.add(e.source_id);
        s.add(e.target_id);
      }
    }
    return s;
  }, [relTypeFilter, edges]);

  // 3) computeDim helper (closed over current filter/highlight)
  function computeDim(n: GraphNode): boolean {
    if (labelFilter !== null) return !n.labels.includes(labelFilter);
    if (relNodes !== null) return !relNodes.has(n.id);
    if (highlight !== null) return !highlight.has(n.id);
    return false;
  }

  // 4) Build xyflow nodes
  const fnodes: Node[] = useMemo(
    () =>
      nodes.map((n) => ({
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
          isSelected: n.id === selectedNodeId,
          isDimmed: computeDim(n),
        },
      })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [nodes, positions, selectedNodeId, hoveredNodeId, labelFilter, relNodes, highlight],
  );

  // 5) Build xyflow edges (with relationship-type labels + filtering)
  const fedges: Edge[] = useMemo(() => {
    return edges.map((r) => {
      const color = getRelationshipColor(r.relationship_type);
      const edgeHighlighted =
        highlight !== null && highlight.has(r.source_id) && highlight.has(r.target_id);

      // Dimming precedence: rel-type filter → label filter (nodes only) →
      // selection highlight.
      let edgeDimmed = false;
      if (relTypeFilter !== null) {
        edgeDimmed = r.relationship_type !== relTypeFilter;
      } else if (labelFilter !== null) {
        edgeDimmed = false; // label filter dims nodes only
      } else if (highlight !== null) {
        edgeDimmed = !edgeHighlighted;
      }

      const showLabel = showEdgeLabels && !edgeDimmed;
      return {
        id: r.id,
        source: r.source_id,
        target: r.target_id,
        type: "default",
        label: showLabel ? r.relationship_type : undefined,
        labelShowBg: true,
        labelBgStyle: { fill: "#ffffff", fillOpacity: 0.82 },
        labelBgPadding: [3, 1] as [number, number],
        labelBgBorderRadius: 3,
        labelStyle: { fontSize: 8.5, fill: color, fontWeight: 600 },
        markerEnd: {
          type: MarkerType.ArrowClosed,
          color,
          width: 12,
          height: 12,
        },
        style: {
          stroke: color,
          strokeWidth: edgeHighlighted ? 2.4 : 1.3,
          opacity: edgeDimmed ? 0.06 : 0.6,
        },
      };
    });
  }, [edges, highlight, labelFilter, relTypeFilter, showEdgeLabels]);

  // 6) fitView on node-set change (double rAF so ReactFlow has digested nodes)
  useEffect(() => {
    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        fitView({ padding: 0.2, maxZoom: 1.5 });
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [nodeIdsKey]);

  return (
    <ReactFlow
      nodes={fnodes}
      edges={fedges}
      nodeTypes={nodeTypes}
      fitView
      minZoom={0.05}
      maxZoom={4}
      proOptions={{ hideAttribution: true }}
      style={{ background: "#fafbfc" }}
      onNodeClick={(_, n) => onNodeClick(n.id)}
      onNodeMouseEnter={(_, n) => onNodeHover(n.id)}
      onNodeMouseLeave={() => onNodeHover(null)}
      onPaneClick={() => onNodeClick("")}
    >
      <Controls showInteractive={false} />
      <MiniMap
        pannable
        zoomable
        nodeColor={(n) => (n.data as { color?: string }).color ?? "#94a3b8"}
        maskColor="rgba(0,0,0,0.04)"
      />
      <Background variant={BackgroundVariant.Dots} gap={22} size={0.8} color="#e2e8f0" />
    </ReactFlow>
  );
}

// ─── Public export — wraps inner in ReactFlowProvider ────────────────────────

export function GraphCanvas(props: GraphCanvasProps) {
  return (
    <ReactFlowProvider>
      <GraphCanvasInner {...props} />
    </ReactFlowProvider>
  );
}
