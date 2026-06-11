import { describe, expect, it } from "vitest";
import Graph from "graphology";
import { sortedNeighbors } from "./graph-walk";

const g = new Graph({ multi: true, type: "directed" });
["hub", "a", "b", "c"].forEach((n) => g.addNode(n, { x: 0, y: 0, size: 1 }));
g.addEdge("hub", "a", { relType: "DEPENDS_ON" });
g.addEdge("hub", "b", { relType: "CONTAINS" });
g.addEdge("c", "hub", { relType: "CONTAINS" });
g.addEdge("a", "b", { relType: "CONTAINS" }); // gives b degree 2 vs a's 2 vs c's 1

describe("sortedNeighbors", () => {
  it("sorts by relationship type, then degree desc, then id", () => {
    const result = sortedNeighbors(g, "hub");
    // CONTAINS (b: deg2, c: deg1) before DEPENDS_ON (a)
    expect(result.map((r) => r.nodeId)).toEqual(["b", "c", "a"]);
    expect(result[0].relType).toBe("CONTAINS");
    expect(result[2].direction).toBe("out");
    expect(result[1].direction).toBe("in");
  });

  it("empty for unknown node", () => {
    expect(sortedNeighbors(g, "ghost")).toEqual([]);
  });
});
