import { describe, expect, it } from "vitest";
import {
  buildSpanTree,
  fmtDurationNs,
  parseAttrs,
  splitOperation,
  utcIso,
  type SpanRow,
} from "./lib";

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
  service_name: "pensieve-server",
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

describe("utcIso", () => {
  it("marks naive server timestamps as UTC", () => {
    // The query API returns timestamps without a timezone suffix; parsing
    // them as local time shifted every span by the user's UTC offset.
    expect(utcIso("2026-06-12T15:19:03.626234")).toBe("2026-06-12T15:19:03.626234Z");
  });
  it("leaves explicit timezones alone", () => {
    expect(utcIso("2026-06-12T15:19:03Z")).toBe("2026-06-12T15:19:03Z");
    expect(utcIso("2026-06-12T15:19:03+03:00")).toBe("2026-06-12T15:19:03+03:00");
    expect(utcIso("2026-06-12T15:19:03.1-07:00")).toBe("2026-06-12T15:19:03.1-07:00");
  });
});

describe("parseAttrs", () => {
  it("parses and stringifies values", () => {
    expect(parseAttrs('{"http.status":"200","n":3}')).toEqual({ "http.status": "200", n: "3" });
  });
  it("tolerates malformed json", () => {
    expect(parseAttrs("not json")).toEqual({});
    expect(parseAttrs("")).toEqual({});
  });
});

describe("splitOperation", () => {
  it("splits HTTP-style names into method + path", () => {
    expect(splitOperation("GET /v1/query")).toEqual({ method: "GET", path: "/v1/query" });
    expect(splitOperation("POST /v1/ingest")).toEqual({ method: "POST", path: "/v1/ingest" });
  });
  it("passes through non-HTTP operation names", () => {
    expect(splitOperation("memory.recall")).toEqual({ method: null, path: "memory.recall" });
    expect(splitOperation("GET something extra")).toEqual({
      method: null,
      path: "GET something extra",
    });
  });
});
