import { describe, expect, it } from "vitest";
import { groupByCommunity } from "./HullsLayer";

describe("groupByCommunity", () => {
  it("returns empty map for empty input", () => {
    const result = groupByCommunity(new Map());
    expect(result.size).toBe(0);
  });

  it("puts nodes with same community index in the same group", () => {
    const assignment = new Map([
      ["a", 0],
      ["b", 0],
      ["c", 1],
      ["d", 1],
      ["e", 0],
    ]);
    const result = groupByCommunity(assignment);
    expect(result.size).toBe(2);
    expect(result.get(0)!.sort()).toEqual(["a", "b", "e"]);
    expect(result.get(1)!.sort()).toEqual(["c", "d"]);
  });

  it("handles all nodes in the same community", () => {
    const assignment = new Map([
      ["x", 3],
      ["y", 3],
      ["z", 3],
    ]);
    const result = groupByCommunity(assignment);
    expect(result.size).toBe(1);
    expect(result.get(3)!.sort()).toEqual(["x", "y", "z"]);
  });

  it("handles all nodes in distinct communities (singletons)", () => {
    const assignment = new Map([
      ["p", 0],
      ["q", 1],
      ["r", 2],
    ]);
    const result = groupByCommunity(assignment);
    expect(result.size).toBe(3);
    expect(result.get(0)).toEqual(["p"]);
    expect(result.get(1)).toEqual(["q"]);
    expect(result.get(2)).toEqual(["r"]);
  });

  it("preserves insertion order within a group", () => {
    // Nodes are visited in Map insertion order; members accumulate in that order.
    const assignment = new Map([
      ["first", 0],
      ["second", 1],
      ["third", 0],
      ["fourth", 0],
    ]);
    const result = groupByCommunity(assignment);
    expect(result.get(0)).toEqual(["first", "third", "fourth"]);
    expect(result.get(1)).toEqual(["second"]);
  });

  it("covers the full round-trip from detectCommunities output", () => {
    // Simulate detectCommunities: a star topology (hub + 4 leaves) should
    // converge to a single community after label propagation.
    // We only test groupByCommunity here — community detection itself has
    // its own tests in graph-community.ts.
    const starAssignment = new Map([
      ["hub", 0],
      ["leaf0", 0],
      ["leaf1", 0],
      ["leaf2", 0],
      ["leaf3", 0],
    ]);
    const result = groupByCommunity(starAssignment);
    expect(result.size).toBe(1);
    expect(result.get(0)!.length).toBe(5);
  });
});
