import { expect, test } from "vitest";
import { partitionColumns, formatCell, stringColumnsOf } from "./columns";

const vec = (n: number) => Array.from({ length: n }, (_, i) => i * 0.01);

test("partitionColumns puts timestamp first and sorts the rest", () => {
  const rows = [{ b: 1, timestamp: "2026-01-01T00:00:00Z", a: 2 }];
  const { shown, hiddenVectors } = partitionColumns(rows);
  expect(shown).toEqual(["timestamp", "a", "b"]);
  expect(hiddenVectors).toEqual([]);
});

test("partitionColumns hides numeric-vector columns like embeddings", () => {
  const rows = [
    { id: "x", embedding: vec(128), content: "hello" },
    { id: "y", embedding: vec(128), content: "world" },
  ];
  const { shown, hiddenVectors } = partitionColumns(rows);
  expect(shown).toEqual(["content", "id"]);
  expect(hiddenVectors).toEqual(["embedding"]);
});

test("partitionColumns keeps short numeric arrays visible", () => {
  const rows = [{ id: "x", grid: [1, 2, 3, 4] }];
  const { shown, hiddenVectors } = partitionColumns(rows);
  expect(shown).toEqual(["grid", "id"]);
  expect(hiddenVectors).toEqual([]);
});

test("partitionColumns treats vector-as-string columns as vectors", () => {
  // Some engines serialize vectors to a JSON string before they reach the UI.
  const rows = [{ id: "x", embedding: JSON.stringify(vec(64)) }];
  const { hiddenVectors } = partitionColumns(rows);
  expect(hiddenVectors).toEqual(["embedding"]);
});

test("formatCell truncates very long values", () => {
  const s = "x".repeat(500);
  const out = formatCell(s);
  expect(out.length).toBeLessThanOrEqual(201);
  expect(out.endsWith("…")).toBe(true);
});

test("formatCell renders objects as JSON and nulls as empty", () => {
  expect(formatCell({ a: 1 })).toBe('{"a":1}');
  expect(formatCell(null)).toBe("");
  expect(formatCell(42)).toBe("42");
});

// ── stringColumnsOf ──────────────────────────────────────────────────────

const mkVec = (n: number) => Array.from({ length: n }, (_, i) => i * 0.01);

test("stringColumnsOf returns columns whose non-null values are all strings", () => {
  const rows = [
    { m: "hello", svc: "auth", count: 42, active: true },
    { m: "world", svc: "pay", count: 99, active: false },
  ];
  const cols = stringColumnsOf(rows);
  expect(cols).toContain("m");
  expect(cols).toContain("svc");
  expect(cols).not.toContain("count");
  expect(cols).not.toContain("active");
});

test("stringColumnsOf excludes vector columns", () => {
  const rows = [
    { body: "text", embedding: mkVec(128) },
    { body: "more", embedding: mkVec(128) },
  ];
  const cols = stringColumnsOf(rows);
  expect(cols).toContain("body");
  expect(cols).not.toContain("embedding");
});

test("stringColumnsOf tolerates null/undefined values (skips them)", () => {
  const rows: Record<string, unknown>[] = [
    { m: "hello", svc: null },
    { m: "world", svc: "pay" },
  ];
  const cols = stringColumnsOf(rows);
  expect(cols).toContain("m");
  // svc has at least one non-null string value; the null row is skipped
  expect(cols).toContain("svc");
});

test("stringColumnsOf returns empty array for empty rows", () => {
  expect(stringColumnsOf([])).toEqual([]);
});

test("stringColumnsOf excludes a given timestampColumn", () => {
  const rows = [{ ts: "2026-01-01T00:00:00Z", m: "hello" }];
  const cols = stringColumnsOf(rows, "ts");
  expect(cols).toContain("m");
  expect(cols).not.toContain("ts");
});
