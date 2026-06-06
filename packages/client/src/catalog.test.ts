import { beforeEach, expect, test, vi } from "vitest";
import { fetchSchema, type SchemaDoc } from "./catalog";
import { createTransport } from "./transport";

const mockFetch = vi.fn();
beforeEach(() => { mockFetch.mockReset(); });

function makeTransport(fetchMock: typeof fetch, endpoint = "http://localhost:8080") {
  return createTransport({ endpoint, auth: { token: "t" }, fetch: fetchMock });
}

test("fetches and returns the schema tree", async () => {
  const doc: SchemaDoc = {
    databases: [{ name: "obs", tables: [{ name: "otel_logs",
      columns: [{ name: "timestamp", type: "datetime", nullable: false }] }] }],
  };
  mockFetch.mockResolvedValue(new Response(JSON.stringify(doc), {
    status: 200, headers: { "content-type": "application/json" },
  }));
  const got = await fetchSchema(makeTransport(mockFetch));
  expect(got).toEqual(doc);
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/catalog/schema");
  expect(init.headers.get("authorization")).toBe("Bearer t");
});

test("throws Unauthorized on 401", async () => {
  mockFetch.mockResolvedValue(new Response("nope", { status: 401 }));
  await expect(fetchSchema(makeTransport(mockFetch, "http://x"))).rejects.toThrow(/unauthorized|token rejected/i);
});
