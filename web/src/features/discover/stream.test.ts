import { expect, test } from "vitest";
import { mergeSources } from "./stream";
import type { SourceState } from "./types";

function src(over: Partial<SourceState>): SourceState {
  return {
    source: "db.t",
    hasTimestamp: true,
    timestampColumn: "timestamp",
    progress: "done",
    rows: [],
    total: 0,
    capped: false,
    droppedClauses: [],
    ...over,
  };
}

test("mergeSources merges rows across sources sorted by time desc", () => {
  const a = src({
    source: "db.a",
    rows: [
      { timestamp: "2026-06-05T10:00:00Z", m: "a-old" },
      { timestamp: "2026-06-05T12:00:00Z", m: "a-new" },
    ],
  });
  const b = src({
    source: "db.b",
    rows: [{ timestamp: "2026-06-05T11:00:00Z", m: "b-mid" }],
  });
  const out = mergeSources([a, b]);
  expect(out.map((r) => r.row.m)).toEqual(["a-new", "b-mid", "a-old"]);
  expect(out[0].source).toBe("db.a");
  expect(out[0].ts).toBe(Date.parse("2026-06-05T12:00:00Z"));
});

test("mergeSources skips sources without a timestamp column", () => {
  const noTs = src({ source: "db.ref", hasTimestamp: false, timestampColumn: null, rows: [{ id: 1 }] });
  const withTs = src({ rows: [{ timestamp: "2026-06-05T10:00:00Z" }] });
  expect(mergeSources([noTs, withTs])).toHaveLength(1);
});

test("mergeSources tolerates rows with missing/garbage timestamps by sinking them", () => {
  const a = src({
    rows: [
      { timestamp: "2026-06-05T10:00:00Z", m: "good" },
      { timestamp: "not a date", m: "bad" },
      { m: "missing" },
    ],
  });
  const out = mergeSources([a]);
  expect(out[0].row.m).toBe("good");
  expect(out.slice(1).map((r) => r.ts)).toEqual([null, null]);
});

test("mergeSources respects the visible filter", () => {
  const a = src({ source: "db.a", rows: [{ timestamp: "2026-06-05T10:00:00Z" }] });
  const b = src({ source: "db.b", rows: [{ timestamp: "2026-06-05T11:00:00Z" }] });
  const out = mergeSources([a, b], ["db.a"]);
  expect(out).toHaveLength(1);
  expect(out[0].source).toBe("db.a");
});

test("mergeSources treats numeric epoch millis as a valid timestamp", () => {
  const a = src({ rows: [{ timestamp: 1717577000000, m: "epoch-ms" }] });
  const out = mergeSources([a]);
  expect(out[0].ts).toBe(1717577000000);
});

test("mergeSources handles Arrow NaiveDateTime format (no T, no Z)", () => {
  // The backend serializes Timestamp columns via NaiveDateTime debug fmt:
  // "2024-06-05 08:43:20.123456" — space-separated, no zone designator.
  const a = src({ rows: [{ timestamp: "2024-06-05 08:43:20.123456", m: "arrow-fmt" }] });
  const out = mergeSources([a]);
  expect(out).toHaveLength(1);
  expect(out[0].ts).not.toBeNull();
});

test("mergeSources treats zone-less timestamps as UTC", () => {
  const a = src({ rows: [{ timestamp: "2026-06-05T11:19:28" }] });
  const out = mergeSources([a]);
  expect(out[0].ts).toBe(Date.parse("2026-06-05T11:19:28Z"));
});

test("mergeSources treats Arrow space-separated timestamps as UTC", () => {
  const a = src({ rows: [{ timestamp: "2024-06-05 08:43:20.123456" }] });
  const out = mergeSources([a]);
  expect(out[0].ts).toBe(Date.parse("2024-06-05T08:43:20.123Z"));
});
