import { describe, it, expect } from "vitest";
import { applyFrame } from "./discover-store";
import type { DiscoverResultsState } from "./types";

const empty = (): DiscoverResultsState => ({
  status: "idle",
  sources: new Map(),
});

describe("applyFrame", () => {
  it("plan creates a pending entry per source", () => {
    const s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: true }],
    });
    expect(s.status).toBe("running");
    expect(s.sources.get("a.b")?.progress).toBe("pending");
    expect(s.sources.get("a.b")?.hasTimestamp).toBe(true);
    expect(s.sources.get("a.b")?.timestampColumn).toBe(null);
  });

  it("plan preserves timestamp_column when present", () => {
    const s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: true, timestamp_column: "ts" }],
    });
    expect(s.sources.get("a.b")?.timestampColumn).toBe("ts");
  });

  it("source_progress flips to running", () => {
    let s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: false }],
    });
    s = applyFrame(s, { type: "source_progress", source: "a.b", state: "running" });
    expect(s.sources.get("a.b")?.progress).toBe("running");
  });

  it("rows appends rows for a source", () => {
    let s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: false }],
    });
    s = applyFrame(s, { type: "rows", source: "a.b", rows: [{ x: 1 }] });
    s = applyFrame(s, { type: "rows", source: "a.b", rows: [{ x: 2 }] });
    expect(s.sources.get("a.b")?.rows).toEqual([{ x: 1 }, { x: 2 }]);
  });

  it("source_done captures totals + capped + droppedClauses", () => {
    let s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: false }],
    });
    s = applyFrame(s, {
      type: "source_done",
      source: "a.b",
      total: 100,
      capped: true,
      dropped_clauses: [{ reason: "unknown_field" }],
    });
    expect(s.sources.get("a.b")?.total).toBe(100);
    expect(s.sources.get("a.b")?.capped).toBe(true);
    expect(s.sources.get("a.b")?.droppedClauses).toHaveLength(1);
  });

  it("histogram stores buckets", () => {
    let s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: true }],
    });
    s = applyFrame(s, {
      type: "histogram",
      source: "a.b",
      buckets: [{ t: "2026-05-28T12:00:00Z", n: 5 }],
    });
    expect(s.sources.get("a.b")?.histogram).toHaveLength(1);
  });

  it("error frame without source becomes topError", () => {
    const s = applyFrame(empty(), {
      type: "error",
      code: "x",
      message: "y",
    });
    expect(s.topError).toEqual({ code: "x", message: "y" });
  });

  it("error frame with source marks the source as error", () => {
    let s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: false }],
    });
    s = applyFrame(s, {
      type: "error",
      source: "a.b",
      code: "timeout",
      message: "slow",
    });
    expect(s.sources.get("a.b")?.progress).toBe("error");
    expect(s.sources.get("a.b")?.error?.code).toBe("timeout");
  });

  it("done with a topError ends in error state", () => {
    let s = applyFrame(empty(), {
      type: "error",
      code: "x",
      message: "y",
    });
    s = applyFrame(s, { type: "done", elapsed_ms: 1 });
    expect(s.status).toBe("error");
  });

  it("done without topError ends in done state", () => {
    const s = applyFrame(empty(), { type: "done", elapsed_ms: 1 });
    expect(s.status).toBe("done");
  });

  it("live frame after plan sets status to live", () => {
    let s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: true }],
    });
    s = applyFrame(s, { type: "live" });
    expect(s.status).toBe("live");
  });

  it("heartbeat frame is a no-op", () => {
    let s = applyFrame(empty(), {
      type: "plan",
      sources: [{ source: "a.b", has_timestamp: false }],
    });
    const beforeStatus = s.status;
    s = applyFrame(s, { type: "heartbeat" });
    expect(s.status).toBe(beforeStatus);
  });
});
