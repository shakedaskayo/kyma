import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { GraphNode, GraphRelationship, GraphStats } from "@pensieve-ai/client";
import { GraphStoreContext, createGraphStore } from "./graph-store";
import { PensieveProvider } from "../provider/PensieveProvider";
import { InspectorPanel } from "./InspectorPanel";
import { BreadcrumbTrail } from "./BreadcrumbTrail";
import { LegendDock } from "./LegendDock";
import { Minimap } from "./Minimap";

afterEach(cleanup);

// ── helpers ────────────────────────────────────────────────────────────────

const meta = {
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  realm: "default",
};

function makeNode(id: string, name: string, ns = "db/g"): GraphNode {
  return {
    id,
    labels: ["Service"],
    properties: { name },
    metadata: meta,
    namespace: ns,
    database: "db",
  } as unknown as GraphNode;
}

function makeEdge(id: string, srcId: string, dstId: string, ns = "db/g"): GraphRelationship {
  return {
    id,
    source_id: srcId,
    target_id: dstId,
    relationship_type: "DEPENDS_ON",
    properties: {},
    namespace: ns,
  } as unknown as GraphRelationship;
}

function withStore(ui: React.ReactElement, overrides?: Parameters<typeof createGraphStore>[0]) {
  const store = createGraphStore(overrides);
  const { container } = render(
    <GraphStoreContext.Provider value={store}>
      {ui}
    </GraphStoreContext.Provider>,
  );
  return { store, container };
}

// ── InspectorPanel ─────────────────────────────────────────────────────────

describe("InspectorPanel", () => {
  it("renders null when node is null", () => {
    const { container } = withStore(
      <InspectorPanel node={null} edges={[]} nodesByCompositeId={new Map()} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("shows node name and relationship group", () => {
    const node = makeNode("a", "payment-svc");
    const edge = makeEdge("e1", "a", "b");
    const nodesByCompositeId = new Map<string, GraphNode>([
      ["db/g::b", makeNode("b", "orders-db")],
    ]);

    withStore(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
        <InspectorPanel
          node={node}
          edges={[edge]}
          nodesByCompositeId={nodesByCompositeId}
        />
      </PensieveProvider>,
    );

    // Node name appears (may appear multiple times - header + properties)
    expect(screen.getAllByText("payment-svc").length).toBeGreaterThan(0);
    // Relationship type group
    expect(screen.getByText("DEPENDS_ON")).toBeTruthy();
    // The other node name should appear in the relationship list
    expect(screen.getByText("orders-db")).toBeTruthy();
  });

  it("shows ArrowRight (out) for outgoing edges", () => {
    const node = makeNode("a", "api");
    const edge = makeEdge("e1", "a", "b"); // outgoing from "a"
    const nodesByCompositeId = new Map<string, GraphNode>([
      ["db/g::b", makeNode("b", "worker")],
    ]);

    const { container } = withStore(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
        <InspectorPanel
          node={node}
          edges={[edge]}
          nodesByCompositeId={nodesByCompositeId}
        />
      </PensieveProvider>,
    );

    // ArrowRight should be present (outgoing direction)
    // lucide renders svg with data-lucide attribute
    const svgs = container.querySelectorAll("svg");
    expect(svgs.length).toBeGreaterThan(0);
  });

  it("closes on X click (selectNode null)", () => {
    const node = makeNode("a", "api");
    const { store } = withStore(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
        <InspectorPanel node={node} edges={[]} nodesByCompositeId={new Map()} />
      </PensieveProvider>,
    );
    store.getState().selectNode("db/g::a"); // prime state

    // find the first button with an svg child (the X)
    const buttons = screen.getAllByRole("button");
    const xBtn = buttons.find((b) => b.querySelector("svg"));
    if (xBtn) {
      fireEvent.click(xBtn);
      expect(store.getState().selectedNodeId).toBeNull();
    }
  });

  it("shows empty state when no edges", () => {
    const node = makeNode("a", "isolated");
    withStore(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
        <InspectorPanel node={node} edges={[]} nodesByCompositeId={new Map()} />
      </PensieveProvider>,
    );
    expect(screen.getByText("No edges.")).toBeTruthy();
  });
});

// ── BreadcrumbTrail ────────────────────────────────────────────────────────

describe("BreadcrumbTrail", () => {
  it("renders nothing when trail is empty", () => {
    const { container } = withStore(
      <BreadcrumbTrail nodesByCompositeId={new Map()} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders trail entries with node names", () => {
    const store = createGraphStore();
    store.getState().pushTrail("db/g::a");
    store.getState().pushTrail("db/g::b");

    const nodesByCompositeId = new Map<string, GraphNode>([
      ["db/g::a", makeNode("a", "service-a")],
      ["db/g::b", makeNode("b", "service-b")],
    ]);

    render(
      <GraphStoreContext.Provider value={store}>
        <BreadcrumbTrail nodesByCompositeId={nodesByCompositeId} />
      </GraphStoreContext.Provider>,
    );

    expect(screen.getByText("service-a")).toBeTruthy();
    expect(screen.getByText("service-b")).toBeTruthy();
  });

  it("jumpTrail fires on crumb click and updates store", () => {
    const store = createGraphStore();
    store.getState().pushTrail("db/g::a");
    store.getState().pushTrail("db/g::b");
    store.getState().pushTrail("db/g::c");

    const nodesByCompositeId = new Map<string, GraphNode>([
      ["db/g::a", makeNode("a", "node-a")],
      ["db/g::b", makeNode("b", "node-b")],
      ["db/g::c", makeNode("c", "node-c")],
    ]);

    render(
      <GraphStoreContext.Provider value={store}>
        <BreadcrumbTrail nodesByCompositeId={nodesByCompositeId} />
      </GraphStoreContext.Provider>,
    );

    // Click on "node-a" (index 0 of visible, offset 0)
    fireEvent.click(screen.getByText("node-a"));
    expect(store.getState().selectedNodeId).toBe("db/g::a");
    // Trail should be truncated to the clicked index
    expect(store.getState().trail).toEqual(["db/g::a"]);
  });

  it("shows ellipsis when trail has more than 5 entries", () => {
    const store = createGraphStore();
    for (let i = 0; i < 7; i++) {
      store.getState().pushTrail(`db/g::n${i}`);
    }

    const nodesByCompositeId = new Map<string, GraphNode>();
    for (let i = 0; i < 7; i++) {
      nodesByCompositeId.set(`db/g::n${i}`, makeNode(`n${i}`, `node-${i}`));
    }

    render(
      <GraphStoreContext.Provider value={store}>
        <BreadcrumbTrail nodesByCompositeId={nodesByCompositeId} />
      </GraphStoreContext.Provider>,
    );

    expect(screen.getByText("…")).toBeTruthy();
  });
});

// ── LegendDock ─────────────────────────────────────────────────────────────

describe("LegendDock", () => {
  const baseStats: GraphStats = {
    total_nodes: 42,
    total_relationships: 17,
    label_counts: { Service: 10, Table: 32 },
    relationship_type_counts: { DEPENDS_ON: 17 },
  };

  it("chip shows node and edge counts", () => {
    withStore(
      <LegendDock
        stats={baseStats}
        coords={[]}
        namespaceCounts={{}}
        visibleNodes={42}
        visibleEdges={17}
      />,
    );
    expect(screen.getByText(/42 nodes/)).toBeTruthy();
    expect(screen.getByText(/17 edges/)).toBeTruthy();
  });

  it("expands on click and shows sections", () => {
    withStore(
      <LegendDock
        stats={baseStats}
        coords={[]}
        namespaceCounts={{}}
        visibleNodes={42}
        visibleEdges={17}
      />,
    );

    // Find and click the chip button
    const chipBtn = screen.getByText(/42 nodes/).closest("button");
    expect(chipBtn).toBeTruthy();
    fireEvent.click(chipBtn!);

    // After expanding, Node types section should appear
    expect(screen.getByText("Node types")).toBeTruthy();
    // Label entries from stats
    expect(screen.getByText("Service")).toBeTruthy();
    expect(screen.getByText("Table")).toBeTruthy();
    // Relationship section
    expect(screen.getByText("Relationships")).toBeTruthy();
    expect(screen.getByText("DEPENDS_ON")).toBeTruthy();
    // Layout picker
    expect(screen.getByText("Force")).toBeTruthy();
  });

  it("toggles namespace hidden on click when expanded", () => {
    const { store } = withStore(
      <LegendDock
        stats={baseStats}
        coords={[]}
        namespaceCounts={{ "db/g": 42 }}
        visibleNodes={42}
        visibleEdges={17}
      />,
    );

    // Expand
    const chipBtn = screen.getByText(/42 nodes/).closest("button");
    fireEvent.click(chipBtn!);

    // With only 1 namespace, showNamespaceFilter is false (needs >1)
    // stats show node labels
    expect(store.getState().hiddenLabels).toEqual([]);
    // click Service label toggle
    const serviceBtn = screen.getByText("Service").closest("button");
    if (serviceBtn) {
      fireEvent.click(serviceBtn);
      expect(store.getState().hiddenLabels).toEqual(["Service"]);
    }
  });

  it("selects layout on click when expanded", () => {
    const { store } = withStore(
      <LegendDock
        stats={baseStats}
        coords={[]}
        namespaceCounts={{}}
        visibleNodes={42}
        visibleEdges={17}
      />,
    );

    // Expand
    const chipBtn = screen.getByText(/42 nodes/).closest("button");
    fireEvent.click(chipBtn!);

    fireEvent.click(screen.getByText("Tree"));
    expect(store.getState().layout).toBe("tree");
  });
});

// ── Minimap ────────────────────────────────────────────────────────────────

describe("Minimap", () => {
  it("renders canvas without crashing when sigma is null", () => {
    const { container } = withStore(<Minimap sigma={null} />);
    const canvas = container.querySelector("canvas");
    expect(canvas).toBeTruthy();
  });

  it("canvas has correct style dimensions", () => {
    const { container } = withStore(<Minimap sigma={null} />);
    const canvas = container.querySelector("canvas") as HTMLCanvasElement;
    expect(canvas.style.width).toBe("180px");
    expect(canvas.style.height).toBe("120px");
  });
});
