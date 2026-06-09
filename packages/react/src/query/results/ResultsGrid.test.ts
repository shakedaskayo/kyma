import { expect, test } from "vitest";
import { fmtIso } from "./ResultsGrid";

test("fmtIso formats a UTC ISO timestamp as MMM dd HH:mm:ss in local time", () => {
  // Use a fixed timestamp: 2024-04-21T10:17:04.000Z
  // We just assert structure rather than exact value because local tz may differ.
  const result = fmtIso("2024-04-21T10:17:04.000Z");
  expect(result).toMatch(/^[A-Z][a-z]{2} \d{2} \d{2}:\d{2}:\d{2}$/);
});

test("fmtIso returns original string for invalid date", () => {
  expect(fmtIso("not-a-date")).toBe("not-a-date");
  expect(fmtIso("")).toBe("");
});

test("fmtIso returns formatted string for a valid ISO string", () => {
  const input = "2025-01-15T08:05:03.000Z";
  const result = fmtIso(input);
  // Should have 3-letter month, 2-digit day, and HH:mm:ss
  const parts = result.split(" ");
  expect(parts).toHaveLength(3);
  expect(parts[0]).toMatch(/^[A-Z][a-z]{2}$/);
  expect(parts[1]).toMatch(/^\d{2}$/);
  expect(parts[2]).toMatch(/^\d{2}:\d{2}:\d{2}$/);
});
