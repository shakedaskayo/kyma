import { describe, it, expect, vi } from "vitest";
import { search, type HybridSearchRequest, type HybridSearchResponse } from "./search";
import { createTransport } from "./transport";

// ── helpers ─────────────────────────────────────────────────────────────────

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

// ── type-level assertions ────────────────────────────────────────────────────
// These are compile-time checks only; they run at import time.  If the types
// are wrong, `tsc --noEmit` (or vitest's type-check pass) will fail here.

// A memory-mode request must type-check.
void ((): HybridSearchRequest => ({
  query: "error spike",
  scope: { kind: "all" },
  mode: "memory",
  realms: ["logs"],
  memory_type: "observation",
  tags: ["prod"],
  importance_min: 0.5,
  as_of: "2026-01-01T00:00:00Z",
  include_invalidated: false,
}))();

// A graph-mode request must type-check.
void ((): HybridSearchRequest => ({
  query: "AuthService",
  scope: { kind: "all" },
  mode: "graph",
  graph: "default",
  labels: ["Service", "Route"],
  expand_hops: 2,
}))();

// scope is optional — bare query alone must type-check.
void ((): HybridSearchRequest => ({
  query: "hello",
}))();

// A response with all unified-envelope optional fields must be assignable.
void ((): HybridSearchResponse => ({
  hits: [
    {
      source: "pensieve.logs",
      score: 0.9,
      row: { msg: "ok" },
      kind: "memory",
      id: "mem-1",
      title: "Error spike",
      content_preview: "CPU hit 95%…",
      memory_type: "observation",
    },
  ],
  sources_searched: 1,
  elapsed_ms: 42,
  mode: "memory",
  context: "Recent system anomalies",
  linked: [{ id: "mem-2" }],
}))();

// ── runtime behaviour ────────────────────────────────────────────────────────

describe("search()", () => {
  it("sends mode + memory fields in the POST body", async () => {
    const payload: HybridSearchResponse = {
      hits: [],
      sources_searched: 0,
      elapsed_ms: 5,
      mode: "memory",
    };
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(payload));
    const t = createTransport({ endpoint: "https://pensieve.test", auth: { token: "tok" }, fetch: fetchMock });

    const req: HybridSearchRequest = {
      query: "error",
      scope: { kind: "all" },
      mode: "memory",
      realms: ["logs"],
      tags: ["prod"],
      importance_min: 0.8,
    };

    const result = await search(t, req);

    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("https://pensieve.test/v1/search");
    expect(init.method).toBe("POST");

    const body = JSON.parse(init.body as string) as HybridSearchRequest;
    expect(body.mode).toBe("memory");
    expect(body.realms).toEqual(["logs"]);
    expect(body.tags).toEqual(["prod"]);
    expect(body.importance_min).toBe(0.8);

    // Response envelope typed correctly
    expect(result.mode).toBe("memory");
    expect(result.hits).toHaveLength(0);
  });

  it("sends graph fields in the POST body", async () => {
    const payload: HybridSearchResponse = {
      hits: [{ source: "pensieve.graph", score: 0.7, row: {}, kind: "node", id: "n1", title: "AuthService" }],
      sources_searched: 1,
      elapsed_ms: 12,
      mode: "graph",
      linked: [{ id: "n2" }],
    };
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(payload));
    const t = createTransport({ endpoint: "https://pensieve.test", auth: { token: "tok" }, fetch: fetchMock });

    const req: HybridSearchRequest = {
      query: "AuthService",
      mode: "graph",
      graph: "default",
      labels: ["Service"],
      expand_hops: 1,
    };

    const result = await search(t, req);

    const body = JSON.parse((fetchMock.mock.calls[0] as [string, RequestInit])[1].body as string) as HybridSearchRequest;
    expect(body.mode).toBe("graph");
    expect(body.graph).toBe("default");
    expect(body.labels).toEqual(["Service"]);
    expect(body.expand_hops).toBe(1);

    expect(result.mode).toBe("graph");
    expect(result.linked).toEqual([{ id: "n2" }]);
    expect(result.hits[0].kind).toBe("node");
    expect(result.hits[0].title).toBe("AuthService");
  });

  it("backward-compatible: data mode works without new fields", async () => {
    const payload: HybridSearchResponse = {
      hits: [{ source: "db.logs", score: 0.95, row: { msg: "ok" } }],
      sources_searched: 1,
      elapsed_ms: 8,
    };
    const fetchMock = vi.fn().mockResolvedValue(jsonResponse(payload));
    const t = createTransport({ endpoint: "https://pensieve.test", auth: { token: "tok" }, fetch: fetchMock });

    const req: HybridSearchRequest = {
      query: "ok",
      scope: { kind: "all" },
    };

    const result = await search(t, req);

    const body = JSON.parse((fetchMock.mock.calls[0] as [string, RequestInit])[1].body as string) as HybridSearchRequest;
    expect(body.mode).toBeUndefined();

    expect(result.mode).toBeUndefined();
    expect(result.context).toBeUndefined();
    expect(result.linked).toBeUndefined();
    expect(result.hits[0].kind).toBeUndefined();
    expect(result.hits[0].source).toBe("db.logs");
  });
});
