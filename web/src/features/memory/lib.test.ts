import { describe, expect, it } from "vitest";
import { isBackfill, kindStyle, spanRowToEvent, STALL_THRESHOLD_MS } from "./lib";

describe("isBackfill", () => {
  const now = Date.parse("2026-06-11T12:00:00Z");
  it("flags rows older than 5 minutes at arrival", () => {
    expect(isBackfill("2026-06-11T11:54:00Z", now)).toBe(true);
    expect(isBackfill("2026-06-11T11:58:00Z", now)).toBe(false);
  });
  it("treats unparseable timestamps as backfill (never animate junk)", () => {
    expect(isBackfill("not-a-date", now)).toBe(true);
  });
});

describe("spanRowToEvent", () => {
  it("maps an otel_traces row to a pulse event", () => {
    const ev = spanRowToEvent({
      start_time: "2026-06-11T11:59:58Z",
      end_time: "2026-06-11T11:59:59Z",
      name: "memory.recall",
      subject: "ws-mbp-shaked",
      trace_id: "aabb",
      duration_ns: 1_000_000_000,
      attributes_json: JSON.stringify({ "memory.query": "okta sso", "memory.results": "7" }),
    });
    expect(ev.kind).toBe("memory.recall");
    expect(ev.ts).toBe("2026-06-11T11:59:59Z");
    expect(ev.sessionId).toBe("ws-mbp-shaked");
    expect(ev.text).toContain("okta sso");
  });
  it("falls back to trace id when subject is missing", () => {
    const ev = spanRowToEvent({ end_time: "2026-06-11T11:59:59Z", name: "agent.query", trace_id: "deadbeef" });
    expect(ev.sessionId).toBe("deadbeef");
  });
});

describe("kindStyle for op kinds", () => {
  it("has dedicated styles for memory ops", () => {
    for (const k of ["memory.recall", "memory.import", "memory.export", "agent.query", "ingest.batch"]) {
      expect(kindStyle(k).label).not.toBe(k); // mapped, not the raw fallback
    }
  });
});

it("stall threshold is 10 minutes", () => {
  expect(STALL_THRESHOLD_MS).toBe(10 * 60 * 1000);
});
