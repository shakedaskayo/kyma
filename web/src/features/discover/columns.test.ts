import { expect, test } from "vitest";
import { partitionColumns, formatCell } from "./columns";

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
