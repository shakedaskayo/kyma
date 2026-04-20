// web/src/sdk/query.test.ts — smoke-level; integration test covered in E2E.
import { expect, test } from "vitest";
import { encodeTicket, reframe } from "./query";

test("encodeTicket serializes database+query+language as JSON", () => {
  const bytes = encodeTicket({ database: "obs", query: "otel_logs | take 1", language: "kql" });
  const text = new TextDecoder().decode(bytes);
  const json = JSON.parse(text);
  expect(json).toEqual({ database: "obs", query: "otel_logs | take 1", language: "kql" });
});

test("reframe emits IPC-stream framing for a FlightData message", () => {
  const msg = {
    dataHeader: new Uint8Array([1, 2, 3]),  // 3 bytes → padded to 8
    dataBody:   new Uint8Array([10, 20, 30, 40]),
  } as any; // structural typing, not the real FlightData class
  const parts = reframe(msg);
  const flat = parts.reduce((acc, p) => { const o = new Uint8Array(acc.length + p.length); o.set(acc); o.set(p, acc.length); return o; }, new Uint8Array());
  // 8-byte prefix
  expect(flat[0]).toBe(0xff);
  expect(flat[1]).toBe(0xff);
  expect(flat[2]).toBe(0xff);
  expect(flat[3]).toBe(0xff);
  // padded length = 8
  const len = new DataView(flat.buffer, 4, 4).getInt32(0, true);
  expect(len).toBe(8);
  // Total = 8 (prefix) + 8 (padded header) + 4 (body) = 20
  expect(flat.length).toBe(20);
});
