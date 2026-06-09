import { beforeEach, expect, test, vi } from "vitest";
import { encodeTicket, runQuery } from "./query";
import { createTransport } from "./transport";

const mockFetch = vi.fn();
beforeEach(() => { mockFetch.mockReset(); });

function makeTransport(fetchMock: typeof fetch) {
  return createTransport({ endpoint: "http://localhost:7070", auth: { token: "t" }, fetch: fetchMock });
}

test("encodeTicket serializes database+query+language as JSON", () => {
  const bytes = encodeTicket({ database: "obs", query: "otel_logs | take 1", language: "kql" });
  const text = new TextDecoder().decode(bytes);
  const json = JSON.parse(text);
  expect(json).toEqual({ database: "obs", query: "otel_logs | take 1", language: "kql" });
});

test("runQuery parses NDJSON and infers column kinds", async () => {
  const ndjson = [
    JSON.stringify({ timestamp: "2026-04-21T10:00:00", n: 1, label: "a" }),
    JSON.stringify({ timestamp: "2026-04-21T10:01:00", n: 2, label: "b" }),
  ].join("\n");
  mockFetch.mockResolvedValue(new Response(ndjson, {
    status: 200, headers: { "content-type": "application/x-ndjson" },
  }));

  const chunks: { columns: unknown; rows: unknown[] }[] = [];
  for await (const c of runQuery(makeTransport(mockFetch), {
    database: "default",
    query: "ev | take 2", language: "kql",
  })) chunks.push(c);

  expect(chunks.length).toBeGreaterThanOrEqual(1);
  const last = chunks.at(-1)!;
  expect(last.rows).toEqual([
    { timestamp: "2026-04-21T10:00:00", n: 1, label: "a" },
    { timestamp: "2026-04-21T10:01:00", n: 2, label: "b" },
  ]);
  expect(last.columns).toEqual([
    { name: "timestamp", kind: "time" },
    { name: "n", kind: "numeric" },
    { name: "label", kind: "string" },
  ]);
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/query");
  expect(init.method).toBe("POST");
  expect(init.headers.get("content-type")).toBe("application/x-kql");
  expect(init.headers.get("x-database")).toBe("default");
  expect(init.headers.get("authorization")).toBe("Bearer t");
  expect(init.body).toBe("ev | take 2");
});

test("runQuery throws with the body on non-OK status", async () => {
  mockFetch.mockResolvedValue(new Response("parse error near 'wat'", { status: 400 }));
  await expect((async () => {
    for await (const _ of runQuery(makeTransport(mockFetch), {
      database: "default",
      query: "wat", language: "kql",
    })) { void _; }
  })()).rejects.toThrow(/query failed \(400\): parse error/);
});
