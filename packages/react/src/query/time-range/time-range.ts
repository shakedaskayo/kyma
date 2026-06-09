import type { ColumnInfo, SchemaDoc } from "@kyma-ai/client";
import type { TimeRange, TimeRangePreset } from "./time-range-types";

export type { TimeRange, TimeRangePreset };

export function presetToKqlAgo(p: TimeRangePreset): string {
  switch (p) {
    case "5m":  return "ago(5m)";
    case "15m": return "ago(15m)";
    case "1h":  return "ago(1h)";
    case "6h":  return "ago(6h)";
    case "24h": return "ago(24h)";
    case "7d":  return "ago(7d)";
    case "30d": return "ago(30d)";
    case "none":
    case "custom": return "";
  }
}

const PRESET_MS: Partial<Record<TimeRangePreset, number>> = {
  "5m": 5 * 60_000,
  "15m": 15 * 60_000,
  "1h": 60 * 60_000,
  "6h": 6 * 60 * 60_000,
  "24h": 24 * 60 * 60_000,
  "7d": 7 * 24 * 60 * 60_000,
  "30d": 30 * 24 * 60 * 60_000,
};

/** Resolve a TimeRange to absolute ISO bounds, or null for "all time"/invalid. */
function rangeToAbsolute(range: TimeRange): { from: string; to: string } | null {
  if (range.preset === "none") return null;
  if (range.preset === "custom") {
    return range.from && range.to ? { from: range.from, to: range.to } : null;
  }
  const ms = PRESET_MS[range.preset];
  if (!ms) return null;
  const to = new Date();
  const from = new Date(to.getTime() - ms);
  return { from: from.toISOString(), to: to.toISOString() };
}

/** First identifier in the query — the leading table name (or null). */
export function extractLeadingTable(query: string): string | null {
  const m = query.trim().match(/^([A-Za-z_][A-Za-z0-9_]*)/);
  return m ? m[1] : null;
}

/** Strong ISO-8601 string time-column names (kept tight; mirrors the backend). */
const ISO_STRING_TIME_NAMES = ["ts", "timestamp", "event_time", "observed_time", "_time"];

const isTimestampType = (t: string) => /timestamp|datetime/i.test(t);
const isStringType = (t: string) => /string|utf8|text|varchar|char/i.test(t);

export type PickedTimeColumn = { name: string; kind: "timestamp" | "string" };

/**
 * Pick the best time column to filter on, mirroring the backend's Discover
 * detection (`crates/kyma-server/src/discover/compile.rs`):
 *   1. A declared timestamp column other than the reserved `at`.
 *   2. A strongly-named ISO-8601 string column (rescues firehose tables whose
 *      only timestamp column is the reserved-and-empty `at`).
 *   3. Any timestamp column (covers a populated `at`).
 * Returns null when the table has no recognizable time column — the caller then
 * injects NO filter rather than a filter on a column that does not exist.
 */
export function pickTimeColumn(columns: ColumnInfo[] | undefined): PickedTimeColumn | null {
  if (!columns || columns.length === 0) return null;
  let f = columns.find((c) => c.name !== "at" && isTimestampType(c.type));
  if (f) return { name: f.name, kind: "timestamp" };
  f = columns.find(
    (c) => isStringType(c.type) && ISO_STRING_TIME_NAMES.some((n) => c.name.toLowerCase() === n),
  );
  if (f) return { name: f.name, kind: "string" };
  f = columns.find((c) => isTimestampType(c.type));
  return f ? { name: f.name, kind: "timestamp" } : null;
}

/** Columns of `table` from the schema (first matching DB — Discover/unify spans DBs). */
function tableColumns(schema: SchemaDoc | undefined, table: string): ColumnInfo[] {
  if (!schema) return [];
  for (const db of schema.databases) {
    const t = db.tables.find((t) => t.name === table);
    if (t) return t.columns;
  }
  return [];
}

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Splice a time filter into a KQL query, using the *real* time column of the
 * leading table (resolved from `schema`) and the comparison form that matches
 * its type:
 *   - timestamp column → `| where <col> > ago(…)` / `between (datetime() .. datetime())`
 *   - ISO-8601 string  → `| where <col> >= "<fromISO>" and <col> < "<toISO>"`
 *
 * If the leading table has no recognizable time column (or no schema is
 * available), the query is returned unchanged — we never inject a filter on a
 * column that may not exist (which previously produced a hard "No field named
 * timestamp" error on every query). The editor's display text is never mutated.
 */
export function prependTimeFilter(query: string, range: TimeRange, schema?: SchemaDoc): string {
  if (range.preset === "none") return query;

  const table = extractLeadingTable(query);
  const col = table ? pickTimeColumn(tableColumns(schema, table)) : null;
  if (!col) return query;

  // If the user already filtered by that column, respect it.
  const guard = new RegExp(`\\b${escapeRegex(col.name)}\\s*(>=|<=|>|<|between)`, "i");
  if (guard.test(query)) return query;

  let filter: string;
  if (col.kind === "timestamp") {
    if (range.preset === "custom" && range.from && range.to) {
      filter = `| where ${col.name} between (datetime(${range.from}) .. datetime(${range.to}))`;
    } else {
      const ago = presetToKqlAgo(range.preset);
      if (!ago) return query;
      filter = `| where ${col.name} > ${ago}`;
    }
  } else {
    const abs = rangeToAbsolute(range);
    if (!abs) return query;
    filter = `| where ${col.name} >= "${abs.from}" and ${col.name} < "${abs.to}"`;
  }

  // Insert immediately after the leading table name (first non-empty line).
  const lines = query.split("\n");
  const firstNonEmpty = lines.findIndex((l) => l.trim().length > 0);
  if (firstNonEmpty < 0) return `${query}\n${filter}`;
  lines.splice(firstNonEmpty + 1, 0, filter);
  return lines.join("\n");
}
