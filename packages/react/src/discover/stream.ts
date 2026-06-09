// Pure stream-merge logic for the unified Discover timeline.
//
// Sources stream in independently; the page shows one time-sorted (desc)
// stream. Sorting is a full re-sort per update — a few thousand rows is
// milliseconds, and it keeps this trivially correct.

import type { SourceKey, SourceState } from "./types";

export type StreamRow = {
  /** Epoch millis, or null when the row's timestamp failed to parse. */
  ts: number | null;
  source: SourceKey;
  row: Record<string, unknown>;
};

// Backend timestamp columns arrive as zone-less strings (Arrow NaiveDateTime,
// e.g. "2026-06-05T11:19:28" or "2026-06-05 08:43:20.123456") whose instants
// are UTC. Date.parse treats zone-less input as LOCAL time, so anchor them to
// UTC explicitly. Zoned strings (Z or ±hh:mm) pass through untouched.
const ZONELESS = /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?$/;
function parseTimestamp(raw: string): number {
  return ZONELESS.test(raw) ? Date.parse(`${raw.replace(" ", "T")}Z`) : Date.parse(raw);
}

export function mergeSources(
  sources: SourceState[],
  visible?: SourceKey[] | null,
): StreamRow[] {
  const out: StreamRow[] = [];
  for (const s of sources) {
    if (s.timestampColumn == null) continue; // not in timeline
    if (visible != null && !visible.includes(s.source)) continue;
    for (const row of s.rows) {
      const raw = row[s.timestampColumn];
      const parsed =
        typeof raw === "number"
          ? raw // epoch millis pass through
          : typeof raw === "string"
            ? parseTimestamp(raw)
            : NaN;
      out.push({ ts: Number.isNaN(parsed) ? null : parsed, source: s.source, row });
    }
  }
  // Desc by time; unparseable timestamps sink to the bottom.
  out.sort((a, b) => {
    if (a.ts === b.ts) return 0;
    if (b.ts === null) return -1;
    if (a.ts === null) return 1;
    return b.ts - a.ts;
  });
  return out;
}
