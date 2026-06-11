import { describe, expect, it } from "vitest";
import { buildSpanTree, fmtDurationNs, type SpanRow } from "./lib";

const span = (over: Partial<SpanRow>): SpanRow => ({
  start_time: "2026-06-11T12:00:00Z",
  end_time: "2026-06-11T12:00:01Z",
  duration_ns: 1e9,
  trace_id: "t1",
  span_id: "s1",
  parent_span_id: null,
  name: "request",
  kind: "SERVER",
  status_code: "OK",
  status_message: null,
  service_name: "kyma-server",
  subject: "ws-1",
  tenant: "default",
  attributes_json: "{}",
  ...over,
});

describe("buildSpanTree", () => {
  it("nests children under parents, sorted by start", () => {
    const rows = [
      span({ span_id: "child2", parent_span_id: "root", start_time: "2026-06-11T12:00:00.500Z" }),
      span({ span_id: "root" }),
      span({ span_id: "child1", parent_span_id: "root", start_time: "2026-06-11T12:00:00.100Z" }),
    ];
    const roots = buildSpanTree(rows);
    expect(roots).toHaveLength(1);
    expect(roots[0].row.span_id).toBe("root");
    expect(roots[0].children.map((c) => c.row.span_id)).toEqual(["child1", "child2"]);
  });
  it("orphans (missing parent) surface as roots, never dropped", () => {
    const roots = buildSpanTree([span({ span_id: "x", parent_span_id: "gone" })]);
    expect(roots).toHaveLength(1);
  });
});

describe("fmtDurationNs", () => {
  it("scales units", () => {
    expect(fmtDurationNs(950)).toBe("950ns");
    expect(fmtDurationNs(2_500_000)).toBe("2.5ms");
    expect(fmtDurationNs(1_250_000_000)).toBe("1.25s");
  });
});
