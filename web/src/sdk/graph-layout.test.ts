import { expect, test } from "vitest";
import { computeLayout, getLabelColor, getRelationshipColor } from "./graph-layout";

test("grid layout positions every node with finite coords", () => {
  const ids = ["a", "b", "c", "d"];
  const pos = computeLayout("grid", ids.map((id) => ({ id })), [], 800, 600);
  expect(pos.size).toBe(4);
  for (const id of ids) {
    const p = pos.get(id)!;
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  }
});

test("force layout is deterministic for the same input", () => {
  const nodes = [{ id: "a" }, { id: "b" }, { id: "c" }];
  const edges = [{ source_id: "a", target_id: "b" }, { source_id: "b", target_id: "c" }];
  const p1 = computeLayout("force", nodes, edges, 800, 600);
  const p2 = computeLayout("force", nodes, edges, 800, 600);
  expect(p1.get("a")).toEqual(p2.get("a"));
  expect(p1.get("c")).toEqual(p2.get("c"));
});

test("kyma label colors resolve; unknown labels still get a color", () => {
  expect(getLabelColor("Table")).toBe("#7ed957");
  expect(typeof getLabelColor("SomethingUnknown")).toBe("string");
  expect(getRelationshipColor("REFERENCES")).toBe("#94a3b8");
});

test("radial layout positions every node with finite coords", () => {
  const ids = ["a", "b", "c", "d", "e"];
  const pos = computeLayout("radial", ids.map((id) => ({ id })), [], 800, 600);
  expect(pos.size).toBe(5);
  for (const id of ids) {
    const p = pos.get(id)!;
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  }
});
