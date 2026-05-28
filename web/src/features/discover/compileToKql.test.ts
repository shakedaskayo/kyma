import { describe, it, expect } from "vitest";
import { compileToKql } from "./compileToKql";
import type { Pill } from "./types";

describe("compileToKql", () => {
  it("returns empty for empty sources", () => {
    expect(compileToKql([], [], null)).toBe("");
  });

  it("single source emits one pipe", () => {
    expect(compileToKql(["obs.otel_logs"], [], null))
      .toBe("otel_logs | take 500");
  });

  it("multi source emits union", () => {
    const out = compileToKql(["obs.otel_logs", "stg.http_reqs"], [], null);
    expect(out.startsWith("union")).toBe(true);
    expect(out).toContain("otel_logs | take 500");
    expect(out).toContain("http_reqs | take 500");
  });

  it("pills become where clauses", () => {
    const pills: Pill[] = [
      { kind: "eq", field: "svc", value: "pay" },
      { kind: "neq", field: "lvl", value: "INFO" },
      { kind: "cmp", field: "status", op: "gt", value: "500" },
      { kind: "exists", field: "trace_id" },
      { kind: "substring", value: "auth" },
    ];
    const out = compileToKql(["obs.otel_logs"], pills, null);
    expect(out).toContain('svc == "pay"');
    expect(out).toContain('lvl != "INFO"');
    expect(out).toContain("status > 500");
    expect(out).toContain("isnotnull(trace_id)");
    expect(out).toContain('* contains "auth"');
  });

  it("time range appended when provided", () => {
    const out = compileToKql(
      ["obs.otel_logs"],
      [],
      { from: "2026-05-28T13:00:00Z", to: "2026-05-28T14:00:00Z" },
    );
    expect(out).toContain('timestamp >= datetime("2026-05-28T13:00:00Z")');
    expect(out).toContain('timestamp < datetime("2026-05-28T14:00:00Z")');
  });

  it("escapes quotes in values", () => {
    const out = compileToKql(
      ["obs.otel_logs"],
      [{ kind: "eq", field: "msg", value: 'say "hi"' }],
      null,
    );
    expect(out).toContain('msg == "say \\"hi\\""');
  });
});
