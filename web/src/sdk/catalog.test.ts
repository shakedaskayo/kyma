import { beforeEach, expect, test, vi } from "vitest";
import { fetchSchema, type SchemaDoc } from "./catalog";

const mockFetch = vi.fn();
beforeEach(() => { vi.stubGlobal("fetch", mockFetch); mockFetch.mockReset(); });

test("fetches and returns the schema tree", async () => {
  const doc: SchemaDoc = {
    databases: [{ name: "obs", tables: [{ name: "otel_logs",
      columns: [{ name: "timestamp", type: "datetime", nullable: false }] }] }],
  };
  mockFetch.mockResolvedValue(new Response(JSON.stringify(doc), {
    status: 200, headers: { "content-type": "application/json" },
  }));
  const got = await fetchSchema({ endpoint: "http://localhost:8080", token: "t" });
  expect(got).toEqual(doc);
  expect(mockFetch).toHaveBeenCalledWith(
    "http://localhost:8080/v1/catalog/schema",
    expect.objectContaining({ headers: expect.objectContaining({ authorization: "Bearer t" }) }),
  );
});

test("throws Unauthorized on 401", async () => {
  mockFetch.mockResolvedValue(new Response("nope", { status: 401 }));
  await expect(fetchSchema({ endpoint: "http://x", token: "t" })).rejects.toThrow(/unauthorized/i);
});
