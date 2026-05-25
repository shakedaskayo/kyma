import { beforeEach, expect, test, vi } from "vitest";
import { getOverview, getNode, searchNodes, expandNeighbors, getStats, getSubgraph, listGraphs } from "./graph";

const mockFetch = vi.fn();
beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
});

function ok(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

const base = { endpoint: "http://localhost:7070", token: "tok" };

test("getOverview GETs the overview endpoint with auth + limit", async () => {
  mockFetch.mockResolvedValue(ok({ stats: { total_nodes: 0, total_relationships: 0, label_counts: {}, relationship_type_counts: {} }, nodes: [], edges: [] }));
  await getOverview({ ...base, graph: "schema", limit: 500 });
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/overview?limit=500");
  expect(init.headers.authorization).toBe("Bearer tok");
});

test("getNode encodes the node id in the path", async () => {
  mockFetch.mockResolvedValue(ok({ id: "default::orders", labels: ["Table"], properties: {}, metadata: { created_at: "", updated_at: "", realm: "default" } }));
  await getNode({ ...base, graph: "schema", id: "default::orders" });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/nodes/default%3A%3Aorders");
});

test("searchNodes POSTs a JSON body", async () => {
  mockFetch.mockResolvedValue(ok({ hits: [], total: 0, limit: 20, offset: 0 }));
  await searchNodes({ ...base, graph: "schema", text: "ord" });
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/search");
  expect(init.method).toBe("POST");
  expect(JSON.parse(init.body)).toMatchObject({ text: "ord" });
});

test("expandNeighbors POSTs node_ids + direction", async () => {
  mockFetch.mockResolvedValue(ok({ edges: [], new_node_ids: [] }));
  await expandNeighbors({ ...base, graph: "schema", nodeIds: ["default::orders"], direction: "both" });
  const [, init] = mockFetch.mock.calls[0];
  expect(JSON.parse(init.body)).toMatchObject({ node_ids: ["default::orders"], direction: "both" });
});

test("throws on 401", async () => {
  mockFetch.mockResolvedValue(new Response("no", { status: 401 }));
  await expect(getOverview({ ...base, graph: "schema" })).rejects.toThrow(/unauthorized/i);
});

test("getOverview without limit/realm omits the query string", async () => {
  mockFetch.mockResolvedValue(ok({ stats: { total_nodes: 0, total_relationships: 0, label_counts: {}, relationship_type_counts: {} }, nodes: [], edges: [] }));
  await getOverview({ ...base, graph: "schema" });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/overview");
});

test("getSubgraph encodes the id and sends depth", async () => {
  mockFetch.mockResolvedValue(ok({ stats: { total_nodes: 0, total_relationships: 0, label_counts: {}, relationship_type_counts: {} }, nodes: [], edges: [] }));
  await getSubgraph({ ...base, graph: "schema", id: "default::orders", depth: 3 });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/nodes/default%3A%3Aorders/subgraph?depth=3");
});

test("getStats sends realm query param", async () => {
  mockFetch.mockResolvedValue(ok({ total_nodes: 0, total_relationships: 0, label_counts: {}, relationship_type_counts: {} }));
  await getStats({ ...base, graph: "schema", realm: "default" });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/stats?realm=default");
});

test("listGraphs GETs /v1/graph and returns refs", async () => {
  mockFetch.mockResolvedValue(ok([{ name: "schema", kind: "schema", description: "x" }]));
  const refs = await listGraphs(base);
  expect(refs[0].name).toBe("schema");
});

test("sends x-database header when database is set", async () => {
  mockFetch.mockResolvedValue(ok({ stats: { total_nodes: 0, total_relationships: 0, label_counts: {}, relationship_type_counts: {} }, nodes: [], edges: [] }));
  await getOverview({ ...base, database: "obs", graph: "kg" });
  const [, init] = mockFetch.mock.calls[0];
  expect(init.headers["x-database"]).toBe("obs");
});

test("omits x-database header when database is absent", async () => {
  mockFetch.mockResolvedValue(ok([{ name: "schema", kind: "schema", description: "" }]));
  await listGraphs(base); // base has no database
  const [, init] = mockFetch.mock.calls[0];
  expect(init.headers["x-database"]).toBeUndefined();
});
