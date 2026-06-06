import { expect, test, describe } from "vitest";
import { createGraphStore } from "./graph-store";

describe("createGraphStore factory", () => {
  test("default state", () => {
    const store = createGraphStore();
    const s = store.getState();
    expect(s.graph).toBe("all");
    expect(s.layout).toBe("force");
    expect(s.selectedNodeId).toBeNull();
    expect(s.labelFilter).toBeNull();
  });

  test("accepts initial overrides", () => {
    const store = createGraphStore({ layout: "radial", graph: "mydb/schema" });
    const s = store.getState();
    expect(s.layout).toBe("radial");
    expect(s.graph).toBe("mydb/schema");
  });

  test("select + clear node", () => {
    const store = createGraphStore();
    store.getState().selectNode("default::orders");
    expect(store.getState().selectedNodeId).toBe("default::orders");
    store.getState().selectNode(null);
    expect(store.getState().selectedNodeId).toBeNull();
  });

  test("toggle label filter", () => {
    const store = createGraphStore();
    store.getState().setLabelFilter("Table");
    expect(store.getState().labelFilter).toBe("Table");
    store.getState().setLabelFilter(null);
    expect(store.getState().labelFilter).toBeNull();
  });

  test("setLayout + setGraph", () => {
    const store = createGraphStore();
    store.getState().setLayout("radial");
    expect(store.getState().layout).toBe("radial");
    store.getState().setGraph("schema");
    expect(store.getState().graph).toBe("schema");
  });

  test("reset restores initial state", () => {
    const store = createGraphStore();
    store.getState().selectNode("abc");
    store.getState().setLayout("tree");
    store.getState().reset();
    const s = store.getState();
    expect(s.selectedNodeId).toBeNull();
    expect(s.layout).toBe("force");
  });

  test("two instances are independent", () => {
    const store1 = createGraphStore();
    const store2 = createGraphStore();
    store1.getState().selectNode("node-A");
    store1.getState().setLayout("radial");
    // store2 must be unaffected
    expect(store2.getState().selectedNodeId).toBeNull();
    expect(store2.getState().layout).toBe("force");
  });

  test("toggleHiddenLabel adds and removes", () => {
    const store = createGraphStore();
    store.getState().toggleHiddenLabel("Table");
    expect(store.getState().hiddenLabels).toContain("Table");
    store.getState().toggleHiddenLabel("Table");
    expect(store.getState().hiddenLabels).not.toContain("Table");
  });
});
