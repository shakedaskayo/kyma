import { describe, it, expect, test } from "vitest";
import { detectMode, modeLabel } from "./detectMode";

const TABLES = ["claude_code_events", "events", "otel_logs"];

test("empty is search", () => {
  expect(detectMode("", TABLES)).toBe("search");
  expect(detectMode("   ", TABLES)).toBe("search");
});

test("plain keywords are search", () => {
  expect(detectMode("auth", TABLES)).toBe("search");
  expect(detectMode("error timeout", TABLES)).toBe("search");
  expect(detectMode("service:payments -severity:INFO", TABLES)).toBe("search");
});

test("a pipe means KQL", () => {
  expect(detectMode("claude_code_events | take 50", TABLES)).toBe("kql");
  expect(detectMode("foo | where x > 1", TABLES)).toBe("kql");
});

test("SELECT/WITH/SHOW mean SQL", () => {
  expect(detectMode("SELECT * FROM events", TABLES)).toBe("sql");
  expect(detectMode("  with x as (select 1) select * from x", TABLES)).toBe("sql");
  expect(detectMode("show tables", TABLES)).toBe("sql");
});

test("a bare known table name is KQL (table scan)", () => {
  expect(detectMode("claude_code_events", TABLES)).toBe("kql");
  expect(detectMode("events", TABLES)).toBe("kql");
});

test("a known table followed by a KQL operator is KQL", () => {
  expect(detectMode("events where x == 1", TABLES)).toBe("kql");
  expect(detectMode("events take 10", TABLES)).toBe("kql");
});

test("an unknown word is search even if it looks tabley", () => {
  expect(detectMode("payments", TABLES)).toBe("search");
});

test("a known table name used as a keyword phrase stays search", () => {
  // More than just the table name, not a KQL operator → treat as keyword phrase.
  expect(detectMode("events from yesterday", TABLES)).toBe("search");
});

describe("detectMode cypher", () => {
  it("detects MATCH as cypher", () => { expect(detectMode("MATCH (a)-[r]->(b) RETURN a")).toBe("cypher"); });
  it("detects OPTIONAL MATCH as cypher", () => { expect(detectMode("OPTIONAL MATCH (a) RETURN a")).toBe("cypher"); });
  it("is case-insensitive", () => { expect(detectMode("match (a) return a")).toBe("cypher"); });
  it("does not misfire on a table named match-ish", () => { expect(detectMode("matches | count", ["matches"])).toBe("kql"); });
  it("labels cypher", () => { expect(modeLabel("cypher")).toBe("Cypher"); });
});
