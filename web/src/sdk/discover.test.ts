import { describe, it, expect } from "vitest";
import { parseNdjsonStream, type Frame } from "./discover";

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
