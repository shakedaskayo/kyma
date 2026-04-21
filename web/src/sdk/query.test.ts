import { beforeEach, expect, test, vi } from "vitest";
import { encodeTicket, runQuery } from "./query";

beforeEach(() => { vi.unstubAllGlobals(); });

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
  const mockFetch = vi.fn().mockResolvedValue(new Response(ndjson, {
    status: 200, headers: { "content-type": "application/x-ndjson" },
  }));
  vi.stubGlobal("fetch", mockFetch);

  const chunks: { columns: unknown; rows: unknown[] }[] = [];
  for await (const c of runQuery({
    endpoint: "http://localhost:7070", token: "t", database: "default",
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
  expect(mockFetch).toHaveBeenCalledWith(
    "http://localhost:7070/v1/query",
    expect.objectContaining({
      method: "POST",
      headers: expect.objectContaining({
        "content-type": "application/x-kql",
        "x-database": "default",
        "authorization": "Bearer t",
      }),
      body: "ev | take 2",
    }),
  );
});

test("runQuery throws with the body on non-OK status", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("parse error near 'wat'", {
    status: 400,
  })));
  await expect((async () => {
    for await (const _ of runQuery({
      endpoint: "http://x", token: "t", database: "default",
      query: "wat", language: "kql",
    })) { void _; }
  })()).rejects.toThrow(/query failed \(400\): parse error/);
});
