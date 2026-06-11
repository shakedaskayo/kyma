import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import Graph from "graphology";
import { CommandBar } from "./CommandBar";
import { GraphStoreContext, createGraphStore } from "./graph-store";

function setup() {
  const store = createGraphStore({ commandBarOpen: true });
  const graph = new Graph({ multi: true, type: "directed" });
  graph.addNode("db/g::a", { label: "payment-svc", nodeLabel: "Service", x: 0, y: 0, size: 1 });
  graph.addNode("db/g::b", { label: "orders-table", nodeLabel: "Table", x: 0, y: 0, size: 1 });
  const utils = render(
    <GraphStoreContext.Provider value={store}>
      <CommandBar graphRef={{ current: graph }} />
    </GraphStoreContext.Provider>,
  );
  return { store, ...utils };
}

afterEach(cleanup);

describe("CommandBar", () => {
  it("filters nodes by text and selects on click", () => {
    const { store } = setup();
    fireEvent.change(screen.getByPlaceholderText(/search/i), { target: { value: "payment" } });
    expect(screen.getByText("payment-svc")).toBeTruthy();
    expect(screen.queryByText("orders-table")).toBeNull();
    fireEvent.click(screen.getByText("payment-svc"));
    expect(store.getState().selectedNodeId).toBe("db/g::a");
    expect(store.getState().commandBarOpen).toBe(false);
    expect(store.getState().trail).toEqual(["db/g::a"]);
  });

  it("closes on Escape", () => {
    const { store } = setup();
    fireEvent.keyDown(screen.getByPlaceholderText(/search/i), { key: "Escape" });
    expect(store.getState().commandBarOpen).toBe(false);
  });
});
