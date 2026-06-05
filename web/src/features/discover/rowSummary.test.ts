import { expect, test } from "vitest";
import { pickMessageField, summarizeRow } from "./rowSummary";

test("pickMessageField prefers well-known message names", () => {
  const rows = [{ message: "hi", id: "1", note: "a much longer string than message" }];
  expect(pickMessageField(rows)).toBe("message");
});

test("pickMessageField falls back to the longest avg string field", () => {
  const rows = [
    { id: "1", detail: "a fairly long description of the event" },
    { id: "2", detail: "another long description here" },
  ];
  expect(pickMessageField(rows)).toBe("detail");
});

test("pickMessageField returns null for rows with no string fields", () => {
  expect(pickMessageField([{ n: 1, b: true }])).toBe(null);
});

test("summarizeRow returns primary text and remaining k=v pairs", () => {
  const row = { timestamp: "t", message: "boom", service: "api", code: 500 };
  const s = summarizeRow(row, "message", "timestamp", ["code"]);
  expect(s.primary).toBe("boom");
  expect(s.rest).toEqual([["service", "api"]]); // ts column and excluded cols dropped
});

test("summarizeRow without a message field puts everything in rest", () => {
  const s = summarizeRow({ a: 1, b: "x" }, null, null, []);
  expect(s.primary).toBe(null);
  expect(s.rest).toEqual([["a", "1"], ["b", "x"]]);
});

test("pickMessageField returns null for an empty rows array", () => {
  expect(pickMessageField([])).toBe(null);
});

test("pickMessageField skips well-known names holding no string values", () => {
  const rows = [{ message: 42, detail: "a long human readable description" }];
  expect(pickMessageField(rows)).toBe("detail");
});

test("pickMessageField breaks avg-length ties alphabetically", () => {
  expect(pickMessageField([{ beta: "ab" }, { alpha: "ab" }])).toBe("alpha");
});
