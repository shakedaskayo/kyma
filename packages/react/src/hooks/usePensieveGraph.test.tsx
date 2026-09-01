/**
 * usePensieveGraph tests — failing-first TDD.
 *
 * Fetch stubs follow the CapabilityGate recipe:
 *   - vi.stubGlobal("fetch", ...) for the global fetch used by the transport.
 *   - Headers({ "content-type": "application/json" }) on responses where the
 *     transport's html-guard checks content-type.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, cleanup, waitFor } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { PensieveProvider } from "../provider/PensieveProvider";
import { usePensieveGraph } from "./usePensieveGraph";
import type { GraphPayload } from "@pensieve-ai/client";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
}

function wrapper(qc: QueryClient) {
  return ({ children }: { children: React.ReactNode }) => (
    <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
      {children}
    </PensieveProvider>
  );
}

const GRAPH_A: GraphPayload = {
  stats: { total_nodes: 2, total_relationships: 1, label_counts: { Person: 2 }, relationship_type_counts: { KNOWS: 1 } },
  nodes: [
    { id: "n1", labels: ["Person"], properties: { name: "Alice" }, metadata: { created_at: "", updated_at: "", realm: "default" } },
    { id: "n2", labels: ["Person"], properties: { name: "Bob" }, metadata: { created_at: "", updated_at: "", realm: "default" } },
  ],
  edges: [
    { id: "e1", source_id: "n1", target_id: "n2", relationship_type: "KNOWS", properties: {} },
  ],
};

const GRAPH_B: GraphPayload = {
  stats: { total_nodes: 1, total_relationships: 0, label_counts: { Service: 1 }, relationship_type_counts: {} },
  nodes: [
    { id: "s1", labels: ["Service"], properties: { name: "svc" }, metadata: { created_at: "", updated_at: "", realm: "default" } },
  ],
  edges: [],
};

function jsonResponse(body: unknown) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: new Headers({ "content-type": "application/json" }),
    }),
  );
}

describe("usePensieveGraph — two-graph merge", () => {
  it("fetches two graphs and merges nodes/edges with namespace+database tags", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/graphA/overview")) return jsonResponse(GRAPH_A);
        if (url.includes("/graphB/overview")) return jsonResponse(GRAPH_B);
        return jsonResponse([]);
      }),
    );

    const { result } = renderHook(
      () =>
        usePensieveGraph({
          graphs: [
            { database: "db1", graph: "graphA" },
            { database: "db2", graph: "graphB" },
          ],
        }),
      { wrapper: wrapper(qc) },
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });

    expect(result.current.nodes.length).toBe(3);
    expect(result.current.edges.length).toBe(1);

    const nodeA = result.current.nodes.find((n) => n.id === "n1");
    expect(nodeA?.namespace).toBe("db1/graphA");
    expect(nodeA?.database).toBe("db1");

    const nodeB = result.current.nodes.find((n) => n.id === "s1");
    expect(nodeB?.namespace).toBe("db2/graphB");
    expect(nodeB?.database).toBe("db2");
  });

  it("aggregates stats across graphs", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/graphA/overview")) return jsonResponse(GRAPH_A);
        if (url.includes("/graphB/overview")) return jsonResponse(GRAPH_B);
        return jsonResponse([]);
      }),
    );

    const { result } = renderHook(
      () =>
        usePensieveGraph({
          graphs: [
            { database: "db1", graph: "graphA" },
            { database: "db2", graph: "graphB" },
          ],
        }),
      { wrapper: wrapper(qc) },
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });
    expect(result.current.stats?.total_nodes).toBe(3);
    expect(result.current.stats?.total_relationships).toBe(1);
  });

  it("is loading when no data has arrived yet", () => {
    const qc = makeQC();
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise(() => {})));

    const { result } = renderHook(
      () =>
        usePensieveGraph({
          graphs: [{ database: "db1", graph: "graphA" }],
        }),
      { wrapper: wrapper(qc) },
    );

    expect(result.current.isLoading).toBe(true);
    expect(result.current.nodes).toEqual([]);
  });

  it("surfaces error when a fetch fails", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockRejectedValue(new Error("network failure")),
    );

    const { result } = renderHook(
      () => usePensieveGraph({ graphs: [{ database: "db1", graph: "graphA" }] }),
      { wrapper: wrapper(qc) },
    );

    await waitFor(() => expect(result.current.error).toBeTruthy(), { timeout: 3000 });
    expect(result.current.isLoading).toBe(false);
  });
});

describe("usePensieveGraph — expandNode", () => {
  it("expandNode appends new nodes and edges from the expansion response", async () => {
    const qc = makeQC();

    const expandResponse = {
      edges: [
        { id: "e2", source_id: "n1", target_id: "n3", relationship_type: "LIKES", properties: {} },
      ],
      new_node_ids: ["n3"],
    };
    const nodeN3 = {
      id: "n3",
      labels: ["Person"],
      properties: { name: "Charlie" },
      metadata: { created_at: "", updated_at: "", realm: "default" },
    };

    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/graphA/overview")) return jsonResponse(GRAPH_A);
        if (url.includes("/graphA/neighbors")) return jsonResponse(expandResponse);
        if (url.includes("/graphA/nodes/n3")) return jsonResponse(nodeN3);
        return jsonResponse([]);
      }),
    );

    const { result } = renderHook(
      () => usePensieveGraph({ graphs: [{ database: "db1", graph: "graphA" }] }),
      { wrapper: wrapper(qc) },
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });
    expect(result.current.nodes.length).toBe(2);

    await result.current.expandNode("n1");

    // After expansion, n3 should be present
    await waitFor(
      () => expect(result.current.nodes.some((n) => n.id === "n3")).toBe(true),
      { timeout: 3000 },
    );
    expect(result.current.edges.some((e) => e.id === "e2")).toBe(true);
  });
});

describe("usePensieveGraph — load all graphs when args is omitted", () => {
  it("calls listGraphs then loads each graph when graphs arg is omitted", async () => {
    const qc = makeQC();

    const graphList = [{ name: "graphA", kind: "property", description: "" }];

    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        // list graphs endpoint
        if (url.endsWith("/v1/graph")) return jsonResponse(graphList);
        if (url.includes("/graphA/overview")) return jsonResponse(GRAPH_A);
        return jsonResponse([]);
      }),
    );

    const { result } = renderHook(() => usePensieveGraph(), { wrapper: wrapper(qc) });

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });
    expect(result.current.nodes.length).toBe(2);
  });
});

describe("usePensieveGraph — searchNodes", () => {
  it("searchNodes calls the search endpoint and returns hits", async () => {
    const qc = makeQC();
    const hits = { hits: [GRAPH_A.nodes[0]], total: 1, limit: 20, offset: 0 };

    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/graphA/overview")) return jsonResponse(GRAPH_A);
        if (url.includes("/graphA/search")) return jsonResponse(hits);
        return jsonResponse([]);
      }),
    );

    const { result } = renderHook(
      () => usePensieveGraph({ graphs: [{ database: "db1", graph: "graphA" }] }),
      { wrapper: wrapper(qc) },
    );

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });

    const res = await result.current.searchNodes("Alice");
    expect(res.hits).toHaveLength(1);
    expect(res.hits[0].id).toBe("n1");
  });
});
