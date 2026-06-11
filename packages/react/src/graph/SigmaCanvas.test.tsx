import { describe, expect, it, vi } from "vitest";

// Sigma requires WebGL which jsdom does not provide. Mock it out so the pure
// buildGraphologyGraph function (which has no WebGL dependency) can be tested.
vi.mock("sigma", () => ({ default: class Sigma {} }));
vi.mock("@sigma/node-image", () => ({ createNodeImageProgram: () => class {} }));
vi.mock("@sigma/edge-curve", () => ({ EdgeCurvedArrowProgram: class {} }));
vi.mock("sigma/rendering", () => ({ EdgeArrowProgram: class {} }));

import { buildGraphologyGraph } from "./SigmaCanvas";
import { createExportAccumulator, mergeExportPage } from "./graph-export-merge";

const meta = { created_at: "", updated_at: "", realm: "default" };

describe("buildGraphologyGraph", () => {
  it("creates positioned, sized, namespaced nodes and directed edges", () => {
    const acc = createExportAccumulator();
    mergeExportPage(
      acc,
      {
        layout_status: "ready",
        layout_id: "L",
        total_nodes: 2,
        total_edges: 1,
        nodes: [
          {
            id: "a",
            labels: ["Service"],
            properties: { name: "api" },
            metadata: meta,
            x: 5,
            y: 6,
          },
          { id: "b", labels: ["Table"], properties: {}, metadata: meta, x: 7, y: 8 },
        ],
        edges: [
          {
            id: "e1",
            source_id: "a",
            target_id: "b",
            relationship_type: "DEPENDS_ON",
            properties: {},
          },
        ],
        next_cursor: undefined,
      },
      "db/g",
      "db",
    );
    const g = buildGraphologyGraph(acc.nodes, acc.edges, acc.positions, {
      sizeByDegree: true,
      hiddenLabels: [],
      activeNamespaces: new Set(["db/g"]),
    });
    expect(g.order).toBe(2);
    expect(g.size).toBe(1);
    expect(g.getNodeAttribute("db/g::a", "x")).toBe(5);
    expect(g.getNodeAttribute("db/g::a", "label")).toBe("api");
    expect(g.getEdgeAttribute("db/g::e1", "relType")).toBe("DEPENDS_ON");
    // hub sizing: degree-1 nodes get base size
    expect(g.getNodeAttribute("db/g::b", "size")).toBeGreaterThanOrEqual(2.5);
  });

  it("filters hidden labels and namespaces", () => {
    const acc = createExportAccumulator();
    mergeExportPage(
      acc,
      {
        layout_status: "ready",
        layout_id: "L",
        total_nodes: 1,
        total_edges: 0,
        nodes: [{ id: "a", labels: ["Secret"], properties: {}, metadata: meta, x: 0, y: 0 }],
        edges: [],
        next_cursor: undefined,
      },
      "db/g",
      "db",
    );
    const g = buildGraphologyGraph(acc.nodes, acc.edges, acc.positions, {
      sizeByDegree: true,
      hiddenLabels: ["Secret"],
      activeNamespaces: new Set(["db/g"]),
    });
    expect(g.order).toBe(0);
  });

  it("hub-dominant sizing: with sizeByDegree a hub with many edges is larger than a leaf", () => {
    // Star topology: 1 hub connected to 50 leaves.
    // With capDeg = max = 50, leaf degree=1 → size ≈ 4.8, hub → 22, ratio > 4.
    const HUB_LEAF_COUNT = 50;
    const acc = createExportAccumulator();
    const nodes = [
      { id: "hub", labels: ["Service"], properties: { name: "hub-svc" }, metadata: meta, x: 0, y: 0 },
      ...Array.from({ length: HUB_LEAF_COUNT }, (_, i) => ({
        id: `leaf${i}`,
        labels: ["Table"],
        properties: {},
        metadata: meta,
        x: i * 10,
        y: 10,
      })),
    ];
    const edges = Array.from({ length: HUB_LEAF_COUNT }, (_, i) => ({
      id: `e${i}`,
      source_id: "hub",
      target_id: `leaf${i}`,
      relationship_type: "CONTAINS",
      properties: {},
    }));
    mergeExportPage(
      acc,
      { layout_status: "ready", layout_id: "L", total_nodes: nodes.length, total_edges: edges.length, nodes, edges },
      "db/g",
      "db",
    );
    const g = buildGraphologyGraph(acc.nodes, acc.edges, acc.positions, {
      sizeByDegree: true,
      hiddenLabels: [],
      activeNamespaces: new Set(["db/g"]),
    });
    const hubSize = g.getNodeAttribute("db/g::hub", "size") as number;
    const leafSize = g.getNodeAttribute("db/g::leaf0", "size") as number;
    // Hub should be at least 4× bigger than a leaf (amendment: range [2.5, 22])
    expect(hubSize).toBeGreaterThan(leafSize * 4);
    // Hub size capped at 22 (amendment upper bound)
    expect(hubSize).toBeLessThanOrEqual(22);
    // Leaf size at minimum 2.5 (amendment lower bound)
    expect(leafSize).toBeGreaterThanOrEqual(2.5);
  });
});

describe("namespaceTileOffsets + positionExtent (degenerate unified views)", () => {
  it("tiles multiple namespaces apart even when their layouts coincide", async () => {
    const { namespaceTileOffsets } = await import("./sigma-graph-builder");
    const mk = (ns: string) =>
      ({ id: "a", labels: ["Memory"], properties: {}, metadata: meta, namespace: ns } as never);
    const nodes = [mk("db/g1"), mk("db/g2"), mk("db/g3")];
    const positions = new Map([
      ["db/g1::a", { x: 833.9, y: 499.2 }],
      ["db/g2::a", { x: 833.9, y: 499.2 }],
      ["db/g3::a", { x: 833.9, y: 499.2 }],
    ]);
    const tiles = namespaceTileOffsets(nodes, positions);
    expect(tiles.size).toBe(3);
    const placed = [...tiles.entries()].map(([, t]) => ({
      x: 833.9 + t.dx,
      y: 499.2 + t.dy,
    }));
    for (let i = 0; i < placed.length; i++)
      for (let j = i + 1; j < placed.length; j++) {
        const d = Math.hypot(placed[i].x - placed[j].x, placed[i].y - placed[j].y);
        expect(d).toBeGreaterThan(300);
      }
  });

  it("single namespace gets no offsets; positionExtent flags coincident graphs", async () => {
    const { namespaceTileOffsets, positionExtent, buildGraphologyGraph, RADIUS_DST_MAX } =
      await import("./sigma-graph-builder");
    const node = { id: "a", labels: ["Memory"], properties: {}, metadata: meta, namespace: "db/g" } as never;
    expect(namespaceTileOffsets([node], new Map([["db/g::a", { x: 5, y: 5 }]])).size).toBe(0);
    const g = buildGraphologyGraph([node], [], new Map([["db/g::a", { x: 5, y: 5 }]]), {
      sizeByDegree: true,
      hiddenLabels: [],
      activeNamespaces: new Set(["db/g"]),
    });
    expect(positionExtent(g)).toBeLessThan(4 * RADIUS_DST_MAX);
  });
});
