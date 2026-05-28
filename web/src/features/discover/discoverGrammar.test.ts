import { describe, it, expect } from "vitest";
import { parseSearch, serializePills } from "./discoverGrammar";

describe("parseSearch", () => {
  it("parses bare substring", () => {
    expect(parseSearch("auth")).toEqual([{ kind: "substring", value: "auth" }]);
  });
  it("parses quoted phrase", () => {
    expect(parseSearch('"foo bar"')).toEqual([{ kind: "substring", value: "foo bar" }]);
  });
  it("parses eq / neq", () => {
    expect(parseSearch("svc:pay -level:INFO")).toEqual([
      { kind: "eq", field: "svc", value: "pay" },
      { kind: "neq", field: "level", value: "INFO" },
    ]);
  });
  it("parses numeric cmp", () => {
    expect(parseSearch("status:>500 lat:<=10")).toEqual([
      { kind: "cmp", field: "status", op: "gt", value: "500" },
      { kind: "cmp", field: "lat", op: "le", value: "10" },
    ]);
  });
  it("parses exists", () => {
    expect(parseSearch("trace_id:*")).toEqual([{ kind: "exists", field: "trace_id" }]);
  });
  it("round-trips via serializePills", () => {
    const inputs = [
      "auth svc:pay -level:INFO status:>500 trace_id:*",
      '"foo bar" baz',
    ];
    for (const s of inputs) {
      const pills = parseSearch(s);
      const reparsed = parseSearch(serializePills(pills));
      expect(reparsed).toEqual(pills);
    }
  });
  it("throws on unterminated quote", () => {
    expect(() => parseSearch('svc:"foo')).toThrow();
  });
  it("throws on dangling negation", () => {
    expect(() => parseSearch("-")).toThrow();
    expect(() => parseSearch("- foo")).toThrow();
  });
  it("throws on negated bare substring", () => {
    expect(() => parseSearch("-foo")).toThrow();
  });
});
