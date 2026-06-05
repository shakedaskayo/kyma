import { describe, it, expect } from "vitest";
import { compileToKql } from "./compileToKql";
import type { Pill } from "./types";

describe("compileToKql", () => {
  it("returns empty string when given a source with empty key", () => {
    expect(compileToKql({ key: "", timestampColumn: null }, [], null)).toBe("");
  });

  it("single source emits one pipe (strips db prefix)", () => {
    expect(compileToKql({ key: "obs.otel_logs", timestampColumn: null }, [], null))
      .toBe("otel_logs | take 500");
  });

  it("strips db prefix — no db prefix left unchanged", () => {
    expect(compileToKql({ key: "otel_logs", timestampColumn: null }, [], null))
      .toBe("otel_logs | take 500");
  });

  it("time range uses the real timestampColumn when provided", () => {
    const out = compileToKql(
      { key: "obs.events", timestampColumn: "at" },
      [],
      { from: "2026-05-28T13:00:00Z", to: "2026-05-28T14:00:00Z" },
    );
    expect(out).toContain('at >= datetime("2026-05-28T13:00:00Z")');
    expect(out).toContain('at < datetime("2026-05-28T14:00:00Z")');
    expect(out).not.toContain("timestamp");
  });

  it("time range is omitted when timestampColumn is null", () => {
    const out = compileToKql(
      { key: "obs.events", timestampColumn: null },
      [],
      { from: "2026-05-28T13:00:00Z", to: "2026-05-28T14:00:00Z" },
    );
    expect(out).not.toContain("datetime");
    expect(out).toBe("events | take 500");
  });

  it("pills become where clauses (substring expands over stringColumns)", () => {
    const pills: Pill[] = [
      { kind: "eq", field: "svc", value: "pay" },
      { kind: "neq", field: "lvl", value: "INFO" },
      { kind: "cmp", field: "status", op: "gt", value: "500" },
      { kind: "exists", field: "trace_id" },
      { kind: "substring", value: "auth" },
    ];
    const out = compileToKql(
      { key: "obs.otel_logs", timestampColumn: null, stringColumns: ["m", "svc"] },
      pills,
      null,
    );
    expect(out).toContain('svc == "pay"');
    expect(out).toContain('lvl != "INFO"');
    expect(out).toContain("status > 500");
    expect(out).toContain("isnotnull(trace_id)");
    // Substring expands to a disjunction — never emits `* contains`
    expect(out).toContain('(m contains "auth" or svc contains "auth")');
    expect(out).not.toContain("* contains");
  });

  it("escapes quotes in values", () => {
    const out = compileToKql(
      { key: "obs.otel_logs", timestampColumn: null },
      [{ kind: "eq", field: "msg", value: 'say "hi"' }],
      null,
    );
    expect(out).toContain('msg == "say \\"hi\\""');
  });

  it("time range with timestampColumn uses correct column name (timestamp column)", () => {
    const out = compileToKql(
      { key: "default.claude_code_events", timestampColumn: "timestamp" },
      [],
      { from: "2026-01-01T00:00:00Z", to: "2026-01-02T00:00:00Z" },
    );
    expect(out).toContain('timestamp >= datetime("2026-01-01T00:00:00Z")');
    expect(out).toContain('timestamp < datetime("2026-01-02T00:00:00Z")');
  });

  // ── substring fix ────────────────────────────────────────────────────────

  it("substring + stringColumns expands to disjunction over named columns", () => {
    const pills: Pill[] = [{ kind: "substring", value: "auth" }];
    const out = compileToKql(
      { key: "obs.otel_logs", timestampColumn: null, stringColumns: ["m", "svc"] },
      pills,
      null,
    );
    expect(out).toContain('(m contains "auth" or svc contains "auth")');
    expect(out).not.toContain("* contains");
  });

  it("substring + empty stringColumns drops the pill (no * and no contains)", () => {
    const pills: Pill[] = [{ kind: "substring", value: "auth" }];
    const out = compileToKql(
      { key: "obs.otel_logs", timestampColumn: null, stringColumns: [] },
      pills,
      null,
    );
    expect(out).not.toContain("* contains");
    expect(out).not.toContain("contains");
    // pill dropped → no where clause for substring
    expect(out).toBe("otel_logs | take 500");
  });

  it("output never contains '* contains' or 'union' regardless of input", () => {
    const pills: Pill[] = [
      { kind: "substring", value: "error" },
      { kind: "eq", field: "svc", value: "pay" },
    ];
    // With stringColumns populated
    const withCols = compileToKql(
      { key: "obs.logs", timestampColumn: "ts", stringColumns: ["body", "svc"] },
      pills,
      { from: "2026-01-01T00:00:00Z", to: "2026-01-02T00:00:00Z" },
    );
    expect(withCols).not.toContain("* contains");
    expect(withCols).not.toContain("union");

    // With empty stringColumns
    const noCols = compileToKql(
      { key: "obs.logs", timestampColumn: "ts", stringColumns: [] },
      pills,
      { from: "2026-01-01T00:00:00Z", to: "2026-01-02T00:00:00Z" },
    );
    expect(noCols).not.toContain("* contains");
    expect(noCols).not.toContain("union");
  });
});
