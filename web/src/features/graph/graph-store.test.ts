import { beforeEach, expect, test } from "vitest";
import { useGraphStore } from "./graph-store";

beforeEach(() => useGraphStore.getState().reset());

test("default state", () => {
  const s = useGraphStore.getState();
  expect(s.graph).toBe("all");
  expect(s.layout).toBe("force");
  expect(s.selectedNodeId).toBeNull();
  expect(s.labelFilter).toBeNull();
});

test("select + clear node", () => {
  useGraphStore.getState().selectNode("default::orders");
  expect(useGraphStore.getState().selectedNodeId).toBe("default::orders");
  useGraphStore.getState().selectNode(null);
  expect(useGraphStore.getState().selectedNodeId).toBeNull();
});

test("toggle label filter", () => {
  useGraphStore.getState().setLabelFilter("Table");
  expect(useGraphStore.getState().labelFilter).toBe("Table");
  useGraphStore.getState().setLabelFilter(null);
  expect(useGraphStore.getState().labelFilter).toBeNull();
});

test("setLayout + setGraph", () => {
  useGraphStore.getState().setLayout("radial");
  expect(useGraphStore.getState().layout).toBe("radial");
  useGraphStore.getState().setGraph("schema");
  expect(useGraphStore.getState().graph).toBe("schema");
});
