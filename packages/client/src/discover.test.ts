import { describe, it, expect, vi } from "vitest";
import { parseNdjsonStream, searchDiscover, DiscoverError, type Frame } from "./discover";
import { createTransport } from "./transport";

function streamFrom(chunks: string[]): ReadableStream<Uint8Array> {
  const enc = new TextEncoder();
  return new ReadableStream({
    start(c) {
      for (const chunk of chunks) c.enqueue(enc.encode(chunk));
      c.close();
    },
  });
}

describe("parseNdjsonStream", () => {
  it("emits frames split across chunk boundaries", async () => {
    const s = streamFrom([
      '{"type":"plan","sources":[]}\n{"type":"sourc',
      'e_progress","source":"db.t","state":"running"}\n',
      '{"type":"done","elapsed_ms":12}\n',
    ]);
    const out: Frame[] = [];
    for await (const f of parseNdjsonStream(s)) out.push(f);
    expect(out.map(f => f.type)).toEqual(["plan", "source_progress", "done"]);
  });

  it("yields no frames for empty input", async () => {
    const s = streamFrom([""]);
    const out: Frame[] = [];
    for await (const f of parseNdjsonStream(s)) out.push(f);
    expect(out).toHaveLength(0);
  });

  it("handles tail without trailing newline", async () => {
    const s = streamFrom(['{"type":"done","elapsed_ms":1}']);
    const out: Frame[] = [];
    for await (const f of parseNdjsonStream(s)) out.push(f);
    expect(out).toHaveLength(1);
    expect(out[0].type).toBe("done");
  });

  it("throws on malformed JSON", async () => {
    const s = streamFrom(['{"type":"plan"\n']);
    let threw = false;
    try { for await (const _ of parseNdjsonStream(s)) { /* drain */ } } catch { threw = true; }
    expect(threw).toBe(true);
  });
});

// ── searchDiscover (transport-level) ────────────────────────────────────────

/** Build a Response whose body is an NDJSON stream of the given frames. */
function ndjsonResponse(frames: Frame[]): Response {
  const enc = new TextEncoder();
  const lines = frames.map((f) => JSON.stringify(f) + "\n").join("");
  const body = new ReadableStream<Uint8Array>({
    start(c) {
      c.enqueue(enc.encode(lines));
      c.close();
    },
  });
  return new Response(body, {
    status: 200,
    headers: { "content-type": "application/x-ndjson" },
  });
}

/** Build an error-envelope Response (non-ok) with the discover error shape. */
function errorEnvelopeResponse(
  status: number,
  code: string,
  message: string,
): Response {
  return new Response(JSON.stringify({ error: { code, message } }), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const REQ = {
  query: "*",
  scope: { kind: "all" as const },
};

describe("searchDiscover", () => {
  it("happy path: yields frames from the NDJSON stream in order", async () => {
    const frames: Frame[] = [
      { type: "plan", sources: [{ source: "db.logs", has_timestamp: true }] },
      { type: "rows", source: "db.logs", rows: [{ msg: "hello" }] },
      { type: "done", elapsed_ms: 42 },
    ];
    const fetchMock = vi.fn().mockResolvedValue(ndjsonResponse(frames));
    const t = createTransport({ endpoint: "https://kyma.test", auth: { token: "tok" }, fetch: fetchMock });

    const out: Frame[] = [];
    for await (const f of searchDiscover(t, REQ)) out.push(f);

    expect(out.map((f) => f.type)).toEqual(["plan", "rows", "done"]);
    expect((out[2] as Extract<Frame, { type: "done" }>).elapsed_ms).toBe(42);
    expect(fetchMock).toHaveBeenCalledWith(
      "https://kyma.test/v1/explore/search",
      expect.objectContaining({ method: "POST" }),
    );
  });

  it("error envelope: throws DiscoverError with code + message from body", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      errorEnvelopeResponse(422, "invalid_scope", "unknown scope kind"),
    );
    const t = createTransport({ endpoint: "https://kyma.test", auth: { token: "tok" }, fetch: fetchMock });

    let caught: unknown;
    try {
      for await (const _ of searchDiscover(t, REQ)) { /* drain */ }
    } catch (e) {
      caught = e;
    }

    expect(caught).toBeInstanceOf(DiscoverError);
    const err = caught as DiscoverError;
    expect(err.code).toBe("invalid_scope");
    expect(err.message).toBe("unknown scope kind");
    expect(err.status).toBe(422);
  });

  it("non-ok without error envelope: throws DiscoverError with http_<status> code", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response("Service Unavailable", { status: 503 }),
    );
    const t = createTransport({ endpoint: "https://kyma.test", auth: { token: "tok" }, fetch: fetchMock });

    let caught: unknown;
    try {
      for await (const _ of searchDiscover(t, REQ)) { /* drain */ }
    } catch (e) {
      caught = e;
    }

    expect(caught).toBeInstanceOf(DiscoverError);
    expect((caught as DiscoverError).code).toBe("http_503");
    expect((caught as DiscoverError).status).toBe(503);
  });

  it("null body: throws DiscoverError with no_body code", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(null, { status: 200 }),
    );
    const t = createTransport({ endpoint: "https://kyma.test", auth: { token: "tok" }, fetch: fetchMock });

    let caught: unknown;
    try {
      for await (const _ of searchDiscover(t, REQ)) { /* drain */ }
    } catch (e) {
      caught = e;
    }

    expect(caught).toBeInstanceOf(DiscoverError);
    expect((caught as DiscoverError).code).toBe("no_body");
  });
});
