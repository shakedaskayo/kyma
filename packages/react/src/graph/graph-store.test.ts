import { expect, test, describe, it } from "vitest";
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

describe("trail", () => {
  it("appends visited nodes, dedups consecutive, caps at 20", () => {
    const s = createGraphStore();
    s.getState().pushTrail("db/g::a");
    s.getState().pushTrail("db/g::a");
    s.getState().pushTrail("db/g::b");
    expect(s.getState().trail).toEqual(["db/g::a", "db/g::b"]);
    for (let i = 0; i < 30; i++) s.getState().pushTrail(`db/g::n${i}`);
    expect(s.getState().trail.length).toBe(20);
  });

  it("jumpTrail truncates after the target and selects it", () => {
    const s = createGraphStore();
    ["a", "b", "c"].forEach((id) => s.getState().pushTrail(id));
    s.getState().jumpTrail(0);
    expect(s.getState().trail).toEqual(["a"]);
    expect(s.getState().selectedNodeId).toBe("a");
    expect(s.getState().focusSeq).toBeGreaterThan(0); // triggers fly-to
  });
});

describe("focus mode + command bar", () => {
  it("focusModeId set/clear", () => {
    const s = createGraphStore();
    s.getState().setFocusMode("db/g::a");
    expect(s.getState().focusModeId).toBe("db/g::a");
    s.getState().setFocusMode(null);
    expect(s.getState().focusModeId).toBeNull();
  });

  it("commandBarOpen toggles", () => {
    const s = createGraphStore();
    s.getState().setCommandBarOpen(true);
    expect(s.getState().commandBarOpen).toBe(true);
  });

  it("setGraph resets focus mode but keeps the trail", () => {
    const s = createGraphStore();
    s.getState().pushTrail("x");
    s.getState().setFocusMode("x");
    s.getState().setGraph("db/g");
    expect(s.getState().focusModeId).toBeNull();
    expect(s.getState().trail).toEqual(["x"]);
  });
});
