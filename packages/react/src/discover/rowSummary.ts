// Smart row summaries for the unified stream: pick a "message-ish" field to
// lead with, render the rest as dimmed k=v pairs. Vector columns are excluded
// by the caller (see columns.ts partitionColumns).

import { formatCell } from "./columns";

const MESSAGE_NAMES = ["message", "msg", "body", "content", "log", "text"];
const SAMPLE = 20;

export function pickMessageField(rows: Record<string, unknown>[]): string | null {
  const sample = rows.slice(0, SAMPLE);
  if (sample.length === 0) return null;
  const fields = new Set<string>();
  for (const r of sample) for (const k of Object.keys(r)) fields.add(k);

  for (const name of MESSAGE_NAMES) {
    if (fields.has(name) && sample.some((r) => typeof r[name] === "string")) return name;
  }

  // Fallback: the string field with the longest average value.
  let best: string | null = null;
  let bestAvg = 0;
  for (const f of fields) {
    const vals = sample.map((r) => r[f]).filter((v): v is string => typeof v === "string");
    if (vals.length === 0) continue;
    const avg = vals.reduce((s, v) => s + v.length, 0) / vals.length;
    if (avg > bestAvg || (avg === bestAvg && best !== null && f < best)) {
      bestAvg = avg;
      best = f;
    }
  }
  return best;
}

export function summarizeRow(
  row: Record<string, unknown>,
  messageField: string | null,
  timestampColumn: string | null,
  excludeColumns: string[],
): { primary: string | null; rest: [string, string][] } {
  const primary = messageField != null && row[messageField] != null ? formatCell(row[messageField]) : null;
  const rest: [string, string][] = [];
  for (const [k, v] of Object.entries(row)) {
    if (k === messageField || k === timestampColumn || excludeColumns.includes(k)) continue;
    if (v == null) continue;
    rest.push([k, formatCell(v)]);
  }
  rest.sort(([a], [b]) => a.localeCompare(b));
  return { primary, rest };
}
