import { describe, expect, it, vi } from "vitest";
import { createPensieveClient } from "./client";

describe("createPensieveClient", () => {
  it("exposes namespaced methods bound to one transport", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify([]), { status: 200, headers: { "content-type": "application/json" } }),
    );
    const client = createPensieveClient({ endpoint: "https://pensieve.test", auth: { token: "tok" }, fetch: fetchMock });
    await client.graph.listGraphs();
    expect(fetchMock.mock.calls[0][0]).toBe("https://pensieve.test/v1/graph");
  });

  it("withDatabase returns a scoped view sharing auth state", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("[]", { status: 200, headers: { "content-type": "application/json" } }));
    const client = createPensieveClient({ endpoint: "https://k", auth: { token: "t" }, fetch: fetchMock });
    await client.withDatabase("staging").graph.listGraphs();
    expect(new Headers(fetchMock.mock.calls[0][1].headers).get("x-database")).toBe("staging");
  });

  it("withDatabase default loses to an explicit per-request database", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("{}", { status: 200, headers: { "content-type": "application/json" } }));
    const client = createPensieveClient({ endpoint: "https://k", auth: { token: "t" }, fetch: fetchMock });
    await client.withDatabase("staging").transport.request("/v1/x", { database: "prod" });
    expect(new Headers(fetchMock.mock.calls[0][1].headers).get("x-database")).toBe("prod");
  });
});
