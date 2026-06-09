import { expect, test } from "vitest";
import { prependTimeFilter, presetToKqlAgo, pickTimeColumn } from "./time-range";
import type { SchemaDoc } from "@kyma-ai/client";

const schemaWith = (table: string, cols: [string, string][]): SchemaDoc => ({
  databases: [
    { name: "default", tables: [{ name: table, columns: cols.map(([name, type]) => ({ name, type, nullable: true })) }] },
  ],
});

const tsSchema = schemaWith("otel_logs", [["timestamp", "timestamp"], ["message", "string"]]);

test("presetToKqlAgo maps presets", () => {
  expect(presetToKqlAgo("5m")).toBe("ago(5m)");
  expect(presetToKqlAgo("1h")).toBe("ago(1h)");
  expect(presetToKqlAgo("24h")).toBe("ago(24h)");
  expect(presetToKqlAgo("7d")).toBe("ago(7d)");
});

test("prependTimeFilter injects on the real timestamp column from schema", () => {
  const out = prependTimeFilter("otel_logs", { preset: "1h" }, tsSchema);
  expect(out).toContain("| where timestamp > ago(1h)");
});

test("prependTimeFilter leaves user-supplied timestamp filter alone", () => {
  const q = "otel_logs | where timestamp > ago(30m)";
  expect(prependTimeFilter(q, { preset: "1h" }, tsSchema)).toBe(q);
});

test("prependTimeFilter supports custom from/to using between syntax", () => {
  const out = prependTimeFilter(
    "otel_logs",
    { preset: "custom", from: "2026-04-20T14:00:00Z", to: "2026-04-20T15:00:00Z" },
    tsSchema,
  );
  expect(out).toContain(
    "| where timestamp between (datetime(2026-04-20T14:00:00Z) .. datetime(2026-04-20T15:00:00Z))",
  );
  const whereCount = (out.match(/\| where/g) ?? []).length;
  expect(whereCount).toBe(1);
});

test("prependTimeFilter injects nothing when no schema is available", () => {
  // Previously this hardcoded `| where timestamp …` and broke tables without
  // a `timestamp` column. Now: no schema → no injection.
  expect(prependTimeFilter("otel_logs", { preset: "1h" })).toBe("otel_logs");
});

test("prependTimeFilter injects nothing when table has no time column", () => {
  const schema = schemaWith("metrics", [["value", "double"], ["name", "string"]]);
  expect(prependTimeFilter("metrics", { preset: "1h" }, schema)).toBe("metrics");
});

test("prependTimeFilter uses a string range for an ISO-string `ts` column (firehose shape)", () => {
  // Reserved `at` (timestamp, empty) + real ISO time in `ts` (string).
  const schema = schemaWith("claude_code_events", [
    ["at", "timestamp"],
    ["ts", "string"],
    ["kind", "string"],
  ]);
  const out = prependTimeFilter(
    "claude_code_events",
    { preset: "custom", from: "2026-06-01T00:00:00Z", to: "2026-06-08T00:00:00Z" },
    schema,
  );
  expect(out).toContain(
    '| where ts >= "2026-06-01T00:00:00Z" and ts < "2026-06-08T00:00:00Z"',
  );
});

test("prependTimeFilter returns query unchanged for the `none` (all time) preset", () => {
  expect(prependTimeFilter("otel_logs", { preset: "none" }, tsSchema)).toBe("otel_logs");
});

test("pickTimeColumn prefers ISO `ts` over reserved `at`", () => {
  const picked = pickTimeColumn([
    { name: "at", type: "timestamp", nullable: true },
    { name: "ts", type: "string", nullable: true },
  ]);
  expect(picked).toEqual({ name: "ts", kind: "string" });
});

test("pickTimeColumn prefers a declared timestamp column over a string ts", () => {
  const picked = pickTimeColumn([
    { name: "event_time", type: "timestamp", nullable: true },
    { name: "ts", type: "string", nullable: true },
  ]);
  expect(picked).toEqual({ name: "event_time", kind: "timestamp" });
});
