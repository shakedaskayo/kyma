import { expect, test } from "vitest";
import { formatSummary } from "./SummaryLine";

test("formatSummary states sources, window, count and as-of", () => {
  const s = formatSummary({
    sourcesSearched: 4,
    windowLabel: "last 1h",
    eventCount: 1284,
    finishedAt: Date.parse("2026-06-05T12:04:31Z"),
    status: "done",
  });
  expect(s).toMatch(/4 sources/);
  expect(s).toMatch(/last 1h/);
  expect(s).toMatch(/1,284 events/);
  expect(s).toMatch(/as of/);
});

test("formatSummary while running says searching", () => {
  const s = formatSummary({ sourcesSearched: 2, windowLabel: "last 1h", eventCount: 10, finishedAt: null, status: "running" });
  expect(s).toMatch(/searching/i);
});
