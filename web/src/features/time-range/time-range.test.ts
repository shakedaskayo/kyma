import { expect, test } from "vitest";
import { prependTimeFilter, presetToKqlAgo } from "./time-range";

test("presetToKqlAgo maps presets", () => {
  expect(presetToKqlAgo("5m")).toBe("ago(5m)");
  expect(presetToKqlAgo("1h")).toBe("ago(1h)");
  expect(presetToKqlAgo("24h")).toBe("ago(24h)");
  expect(presetToKqlAgo("7d")).toBe("ago(7d)");
});

test("prependTimeFilter injects preamble only when user hasn't already filtered by timestamp", () => {
  const out = prependTimeFilter("otel_logs", { preset: "1h" });
  expect(out).toContain("| where timestamp between (ago(1h) .. now())");
});

test("prependTimeFilter leaves user-supplied timestamp filter alone", () => {
  const q = "otel_logs | where timestamp > ago(30m)";
  expect(prependTimeFilter(q, { preset: "1h" })).toBe(q);
});

test("prependTimeFilter supports custom from/to", () => {
  const out = prependTimeFilter("otel_logs", {
    preset: "custom", from: "2026-04-20T14:00:00Z", to: "2026-04-20T15:00:00Z",
  });
  expect(out).toContain("datetime(2026-04-20T14:00:00Z)");
  expect(out).toContain("datetime(2026-04-20T15:00:00Z)");
});
