import { describe, it, expect } from "vitest";
import { parseSearch, tokenize, extractLeadingTable, type SearchSchema } from "./parseSearch";

const schema: SearchSchema = {
  tables: ["otel_logs", "otel_traces"],
  columnsByTable: {
    otel_logs: ["timestamp", "message", "severity_text", "service_name", "error_code", "trace_id"],
    otel_traces: ["timestamp", "span_id", "service_name", "operation"],
  },
  columnTypes: {
    otel_logs: {
      timestamp: "datetime",
      message: "string",
      severity_text: "string",
      service_name: "string",
      error_code: "string",
      trace_id: "string",
    },
    otel_traces: {
      timestamp: "datetime",
      span_id: "string",
      service_name: "string",
      operation: "string",
    },
  },
};

describe("tokenize", () => {
  it("tokenizes bare words", () => {
    const terms = tokenize("auth error");
    expect(terms).toHaveLength(2);
    expect(terms[0]).toEqual({ kind: "bare", word: "auth", negate: false });
    expect(terms[1]).toEqual({ kind: "bare", word: "error", negate: false });
  });

  it("tokenizes field:value pairs", () => {
    const terms = tokenize("service_name:payments");
    expect(terms).toHaveLength(1);
    expect(terms[0]).toEqual({ kind: "field", field: "service_name", value: "payments", negate: false });
  });

  it("tokenizes negated bare words", () => {
    const terms = tokenize("-debug");
    expect(terms).toHaveLength(1);
    expect(terms[0]).toEqual({ kind: "bare", word: "debug", negate: true });
  });

  it("tokenizes negated field:value", () => {
    const terms = tokenize("-severity_text:INFO");
    expect(terms).toHaveLength(1);
    expect(terms[0]).toEqual({ kind: "field", field: "severity_text", value: "INFO", negate: true });
  });

  it("tokenizes quoted multi-word strings as single bare term", () => {
    const terms = tokenize('"auth failure"');
    expect(terms).toHaveLength(1);
    expect(terms[0]).toEqual({ kind: "bare", word: "auth failure", negate: false });
  });

  it("tokenizes mixed input", () => {
    const terms = tokenize('auth service_name:payments -severity_text:INFO');
    expect(terms).toHaveLength(3);
    expect(terms[0]).toEqual({ kind: "bare", word: "auth", negate: false });
    expect(terms[1]).toEqual({ kind: "field", field: "service_name", value: "payments", negate: false });
    expect(terms[2]).toEqual({ kind: "field", field: "severity_text", value: "INFO", negate: true });
  });
});

describe("parseSearch — empty input", () => {
  it("returns table | take N for empty string", () => {
    expect(parseSearch("", "otel_logs", schema)).toBe("otel_logs | take 500");
  });

  it("returns table | take N for blank whitespace", () => {
    expect(parseSearch("   ", "otel_logs", schema)).toBe("otel_logs | take 500");
  });

  it("respects custom limit", () => {
    expect(parseSearch("", "otel_logs", schema, 100)).toBe("otel_logs | take 100");
  });
});

describe("parseSearch — bare words", () => {
  it("expands a single bare word to contains across all string columns", () => {
    const kql = parseSearch("auth", "otel_logs", schema);
    expect(kql).toContain('message contains "auth"');
    expect(kql).toContain('severity_text contains "auth"');
    expect(kql).toContain('service_name contains "auth"');
    expect(kql).toContain("| where (");
    expect(kql).toContain("| take 500");
  });

  it("AND-joins multiple bare words", () => {
    const kql = parseSearch("auth error", "otel_logs", schema);
    const lines = kql.split("\n");
    // Should have two separate where clauses
    const whereClauses = lines.filter((l) => l.trim().startsWith("| where"));
    expect(whereClauses).toHaveLength(2);
  });

  it("negates a bare word with NOT", () => {
    const kql = parseSearch("-debug", "otel_logs", schema);
    expect(kql).toContain("not (");
    expect(kql).toContain('contains "debug"');
  });
});

describe("parseSearch — field:value", () => {
  it("produces exact match for field:value", () => {
    const kql = parseSearch("service_name:payments", "otel_logs", schema);
    expect(kql).toContain('service_name == "payments"');
  });

  it("produces != for negated field:value", () => {
    const kql = parseSearch("-severity_text:INFO", "otel_logs", schema);
    expect(kql).toContain('severity_text != "INFO"');
  });

  it("handles unknown field gracefully (still produces clause)", () => {
    const kql = parseSearch("unknown_col:val", "otel_logs", schema);
    expect(kql).toContain('unknown_col == "val"');
  });
});

describe("parseSearch — quoted multi-word", () => {
  it("treats quoted string as a single search term", () => {
    const kql = parseSearch('"auth failure"', "otel_logs", schema);
    expect(kql).toContain('contains "auth failure"');
  });
});

describe("parseSearch — AND-ing all terms", () => {
  it("produces multiple where clauses for the spec example", () => {
    const kql = parseSearch("auth service_name:payments -severity_text:INFO", "otel_logs", schema);
    expect(kql).toContain('service_name == "payments"');
    expect(kql).toContain('severity_text != "INFO"');
    expect(kql).toContain('contains "auth"');
    expect(kql).toContain("| take 500");
    // Three separate where clauses
    const whereClauses = kql.split("\n").filter((l) => l.trim().startsWith("| where"));
    expect(whereClauses).toHaveLength(3);
  });
});

describe("extractLeadingTable", () => {
  it("returns the table from a simple query", () => {
    expect(extractLeadingTable("otel_logs | where message == 'hi'")).toBe("otel_logs");
  });

  it("returns null for blank query", () => {
    expect(extractLeadingTable("")).toBeNull();
    expect(extractLeadingTable("   ")).toBeNull();
  });

  it("returns the first identifier even with whitespace", () => {
    expect(extractLeadingTable("  otel_traces | take 10")).toBe("otel_traces");
  });
});
