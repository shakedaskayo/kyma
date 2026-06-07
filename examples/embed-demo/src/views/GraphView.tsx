import { useState } from "react";
import { KymaGraph } from "@kyma-ai/react/graph";
import type { GraphNode } from "@kyma-ai/react/graph";

/**
 * Graph tab: KymaGraph with discover="all-databases".
 * Clicking a node opens a side-panel JSON inspector.
 * Proves onNodeClick + onSelectionChange callbacks work.
 */
export function GraphView() {
  const [selectedNode, setSelectedNode] = useState<GraphNode | null>(null);

  return (
    <div style={{ display: "flex", height: "100%" }}>
      <div style={{ flex: 1, overflow: "hidden" }}>
        <KymaGraph
          discover="all-databases"
          height="100%"
          toolbar
          sidebar
          onNodeClick={(node) => {
            setSelectedNode(node);
          }}
          onSelectionChange={(nodes) => {
            if (nodes.length === 0) setSelectedNode(null);
          }}
        />
      </div>

      {selectedNode && (
        <div className="demo-side-panel">
          <h3>
            Node: {selectedNode.labels[0] ?? "Node"}
            <button
              onClick={() => setSelectedNode(null)}
              style={{
                float: "right",
                background: "none",
                border: "none",
                cursor: "pointer",
                fontSize: 16,
                color: "#64748b",
              }}
            >
              ×
            </button>
          </h3>
          <pre>{JSON.stringify(selectedNode, null, 2)}</pre>
        </div>
      )}
    </div>
  );
}
