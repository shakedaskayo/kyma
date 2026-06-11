import { describe, expect, it } from "vitest";
import { createExportAccumulator, mergeExportPage } from "./graph-export-merge";
import type { GraphExportPage } from "@kyma-ai/client";

const meta = { created_at: "", updated_at: "", realm: "default" };
const pnode = (id: string, x: number, y: number) => ({
  id, labels: ["Table"], properties: {}, metadata: meta, x, y,
});
const edge = (id: string, s: string, t: string) => ({
  id, source_id: s, target_id: t, relationship_type: "REFERENCES", properties: {},
});
const page = (p: Partial<GraphExportPage>): GraphExportPage => ({
  layout_status: "ready", layout_id: "L", total_nodes: 0, total_edges: 0,
  nodes: [], edges: [], ...p,
});

describe("mergeExportPage", () => {
  it("namespaces nodes, records positions by composite id, dedups across pages", () => {
    const acc = createExportAccumulator();
    mergeExportPage(acc, page({ nodes: [pnode("a", 1, 2)] }), "db/g", "db");
    mergeExportPage(acc, page({ nodes: [pnode("a", 1, 2), pnode("b", 3, 4)] }), "db/g", "db");
    expect(acc.nodes).toHaveLength(2);
    expect(acc.nodes[0].namespace).toBe("db/g");
    expect(acc.nodes[0].database).toBe("db");
    expect(acc.positions.get("db/g::a")).toEqual({ x: 1, y: 2 });
    expect(acc.positions.get("db/g::b")).toEqual({ x: 3, y: 4 });
  });

  it("merges edges with namespace and counts stats", () => {
    const acc = createExportAccumulator();
    mergeExportPage(acc, page({ nodes: [pnode("a", 0, 0), pnode("b", 0, 0)] }), "db/g", "db");
    mergeExportPage(acc, page({ edges: [edge("e1", "a", "b"), edge("e1", "a", "b")] }), "db/g", "db");
    expect(acc.edges).toHaveLength(1);
    expect(acc.stats.relationship_type_counts.REFERENCES).toBe(1);
    expect(acc.stats.label_counts.Table).toBe(2);
  });

  it("keeps namespaces separate — same node id in two graphs", () => {
    const acc = createExportAccumulator();
    mergeExportPage(acc, page({ nodes: [pnode("a", 0, 0)] }), "db/g1", "db");
    mergeExportPage(acc, page({ nodes: [pnode("a", 9, 9)] }), "db/g2", "db");
    expect(acc.nodes).toHaveLength(2);
    expect(acc.positions.get("db/g2::a")).toEqual({ x: 9, y: 9 });
  });
});
