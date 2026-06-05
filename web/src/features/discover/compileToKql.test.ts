import { describe, it, expect } from "vitest";
import { compileToKql } from "./compileToKql";
import type { Pill } from "./types";

describe("compileToKql", () => {
  // ── existing single-source tests (unchanged) ─────────────────────────────

  it("returns empty string when given a source with empty key", () => {
    expect(compileToKql({ key: "", timestampColumn: null, stringColumns: [] }, [], null)).toBe("");
  });

  it("single source emits one pipe (strips db prefix)", () => {
    expect(compileToKql({ key: "obs.otel_logs", timestampColumn: null, stringColumns: [] }, [], null))
      .toBe("otel_logs | take 500");
  });

  it("strips db prefix — no db prefix left unchanged", () => {
    expect(compileToKql({ key: "otel_logs", timestampColumn: null, stringColumns: [] }, [], null))
      .toBe("otel_logs | take 500");
  });

  it("time range uses the real timestampColumn when provided", () => {
    const out = compileToKql(
      { key: "obs.events", timestampColumn: "at", stringColumns: [] },
      [],
      { from: "2026-05-28T13:00:00Z", to: "2026-05-28T14:00:00Z" },
    );
    expect(out).toContain('at >= datetime("2026-05-28T13:00:00Z")');
    expect(out).toContain('at < datetime("2026-05-28T14:00:00Z")');
    expect(out).not.toContain("timestamp");
  });

  it("time range is omitted when timestampColumn is null", () => {
    const out = compileToKql(
      { key: "obs.events", timestampColumn: null, stringColumns: [] },
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
      { key: "obs.otel_logs", timestampColumn: null, stringColumns: [] },
      [{ kind: "eq", field: "msg", value: 'say "hi"' }],
      null,
    );
    expect(out).toContain('msg == "say \\"hi\\""');
  });

  it("time range with timestampColumn uses correct column name (timestamp column)", () => {
    const out = compileToKql(
      { key: "default.claude_code_events", timestampColumn: "timestamp", stringColumns: [] },
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

  it("output never contains '* contains' or 'union' regardless of input (single source)", () => {
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

  // ── zero-sources ─────────────────────────────────────────────────────────

  it("zero sources → empty string", () => {
    expect(compileToKql([], [], null)).toBe("");
  });

  // ── multi-source union tests ──────────────────────────────────────────────

  it("two sources produce union withsource=_source shape", () => {
    const sources = [
      { key: "obs.otel_logs", timestampColumn: null, stringColumns: [] },
      { key: "obs.otel_traces", timestampColumn: null, stringColumns: [] },
    ];
    const out = compileToKql(sources, [], null);
    // Must start with union withsource=_source
    expect(out).toMatch(/^union withsource=_source /);
    // Both branches parenthesized
    expect(out).toContain("(otel_logs)");
    expect(out).toContain("(otel_traces)");
    // Single trailing take after the union
    expect(out).toMatch(/\| take 500$/);
    // Only one take
    expect(out.split("| take").length - 1).toBe(1);
  });

  it("db prefix is stripped from every branch", () => {
    const sources = [
      { key: "default.table_a", timestampColumn: null, stringColumns: [] },
      { key: "default.table_b", timestampColumn: null, stringColumns: [] },
    ];
    const out = compileToKql(sources, [], null);
    expect(out).toContain("(table_a)");
    expect(out).toContain("(table_b)");
    expect(out).not.toContain("default.");
  });

  it("per-branch timestampColumn: each branch filters with its OWN ts column", () => {
    const tr = { from: "2026-01-01T00:00:00Z", to: "2026-01-02T00:00:00Z" };
    const sources = [
      { key: "obs.logs", timestampColumn: "ts", stringColumns: [] },
      { key: "obs.events", timestampColumn: "at", stringColumns: [] },
    ];
    const out = compileToKql(sources, [], tr);
    // First branch uses 'ts', second uses 'at'
    expect(out).toContain('ts >= datetime("2026-01-01T00:00:00Z")');
    expect(out).toContain('at >= datetime("2026-01-01T00:00:00Z")');
    expect(out).toContain('ts < datetime("2026-01-02T00:00:00Z")');
    expect(out).toContain('at < datetime("2026-01-02T00:00:00Z")');
  });

  it("branch with null timestampColumn has no time filter; other branch does", () => {
    const tr = { from: "2026-01-01T00:00:00Z", to: "2026-01-02T00:00:00Z" };
    const sources = [
      { key: "obs.logs", timestampColumn: "ts", stringColumns: [] },
      { key: "obs.notimestamp", timestampColumn: null, stringColumns: [] },
    ];
    const out = compileToKql(sources, [], tr);
    // 'ts' branch gets the filter
    expect(out).toContain('ts >= datetime("2026-01-01T00:00:00Z")');
    // 'notimestamp' branch: check its parenthesized block has no datetime
    // The second branch should just be (notimestamp) with no where datetime
    expect(out).toContain("(notimestamp)");
    // There should be exactly one datetime occurrence (from the ts branch)
    expect(out.split("datetime(").length - 1).toBe(2); // two datetime calls in the ts branch
  });

  it("substring expands per-branch using each branch's OWN stringColumns", () => {
    const pills: Pill[] = [{ kind: "substring", value: "error" }];
    const sources = [
      { key: "obs.logs", timestampColumn: null, stringColumns: ["body", "svc"] },
      { key: "obs.traces", timestampColumn: null, stringColumns: ["span_name"] },
    ];
    const out = compileToKql(sources, pills, null);
    // First branch expands over body + svc
    expect(out).toContain('(body contains "error" or svc contains "error")');
    // Second branch expands over span_name
    expect(out).toContain('(span_name contains "error")');
    // Never * contains
    expect(out).not.toContain("* contains");
  });

  it("substring dropped for branch with empty stringColumns; other branch keeps it", () => {
    const pills: Pill[] = [{ kind: "substring", value: "auth" }];
    const sources = [
      { key: "obs.logs", timestampColumn: null, stringColumns: ["body"] },
      { key: "obs.empty", timestampColumn: null, stringColumns: [] },
    ];
    const out = compileToKql(sources, pills, null);
    // First branch: substring expanded
    expect(out).toContain('(body contains "auth")');
    // Second branch: no where for substring (pill dropped for that branch)
    // The second branch is just (empty) without a contains clause
    expect(out).toContain("(empty)");
    // No * contains ever
    expect(out).not.toContain("* contains");
  });

  it("three sources produce three parenthesized branches", () => {
    const sources = [
      { key: "db.a", timestampColumn: null, stringColumns: [] },
      { key: "db.b", timestampColumn: null, stringColumns: [] },
      { key: "db.c", timestampColumn: null, stringColumns: [] },
    ];
    const out = compileToKql(sources, [], null);
    expect(out).toMatch(/^union withsource=_source /);
    expect(out).toContain("(a)");
    expect(out).toContain("(b)");
    expect(out).toContain("(c)");
    // Exactly one trailing take
    expect(out).toMatch(/\| take 500$/);
    expect(out.split("| take").length - 1).toBe(1);
  });

  it("pills are applied to every branch in the union", () => {
    const pills: Pill[] = [
      { kind: "eq", field: "level", value: "error" },
    ];
    const sources = [
      { key: "obs.logs", timestampColumn: null, stringColumns: [] },
      { key: "obs.events", timestampColumn: null, stringColumns: [] },
    ];
    const out = compileToKql(sources, pills, null);
    // Both branches include the eq filter
    expect(out).toContain('logs | where level == "error"');
    expect(out).toContain('events | where level == "error"');
  });

  it("no naked union — every branch is parenthesized", () => {
    const sources = [
      { key: "obs.a", timestampColumn: null, stringColumns: [] },
      { key: "obs.b", timestampColumn: null, stringColumns: [] },
    ];
    const out = compileToKql(sources, [], null);
    // All operands after the header must be surrounded by parens
    // i.e. no pattern like "union withsource=_source a, b" without parens
    const afterHeader = out.replace(/^union withsource=_source /, "");
    // Remove trailing | take 500
    const operandsPart = afterHeader.replace(/ \| take 500$/, "");
    for (const operand of operandsPart.split(", ")) {
      expect(operand).toMatch(/^\(.+\)$/);
    }
  });

  it("single source in array produces same output as single-source object (backwards compat)", () => {
    const obj = compileToKql(
      { key: "obs.logs", timestampColumn: "ts", stringColumns: ["body"] },
      [{ kind: "eq", field: "level", value: "error" }],
      { from: "2026-01-01T00:00:00Z", to: "2026-01-02T00:00:00Z" },
    );
    const arr = compileToKql(
      [{ key: "obs.logs", timestampColumn: "ts", stringColumns: ["body"] }],
      [{ kind: "eq", field: "level", value: "error" }],
      { from: "2026-01-01T00:00:00Z", to: "2026-01-02T00:00:00Z" },
    );
    expect(arr).toBe(obj);
  });
});
