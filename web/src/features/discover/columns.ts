// Column curation for Discover result tables.
//
// Raw rows can carry machine columns — embedding vectors, in particular —
// that render as walls of floats and drown out everything else. The grid
// hides those by default (they stay visible in the row-detail drawer) and
// tells the user how many were hidden.

const VECTOR_MIN_LEN = 8;
const SAMPLE_ROWS = 20;
const MAX_CELL_CHARS = 200;

function isNumericVector(v: unknown): boolean {
  if (Array.isArray(v)) {
    return v.length >= VECTOR_MIN_LEN && v.every((x) => typeof x === "number");
  }
  // Some engines serialize vectors to a JSON string before they reach the UI.
  if (typeof v === "string" && v.startsWith("[")) {
    try {
      return isNumericVector(JSON.parse(v));
    } catch {
      return false;
    }
  }
  return false;
}

/** Split a row sample's columns into grid-visible columns and hidden vector
 * columns. `timestamp` sorts first; the rest is alphabetical. */
export function partitionColumns(rows: Record<string, unknown>[]): {
  shown: string[];
  hiddenVectors: string[];
} {
  const sample = rows.slice(0, SAMPLE_ROWS);
  const seen = new Set<string>();
  for (const r of sample) {
    for (const k of Object.keys(r)) seen.add(k);
  }
  const shown: string[] = [];
  const hiddenVectors: string[] = [];
  for (const col of seen) {
    const values = sample.map((r) => r[col]).filter((v) => v != null);
    const vectorish = values.length > 0 && values.every(isNumericVector);
    (vectorish ? hiddenVectors : shown).push(col);
  }
  shown.sort((a, b) => (a === "timestamp" ? -1 : b === "timestamp" ? 1 : a.localeCompare(b)));
  hiddenVectors.sort();
  return { shown, hiddenVectors };
}

/** Render a cell value as a short string — objects become JSON, very long
 * values are truncated with an ellipsis. */
export function formatCell(v: unknown): string {
  if (v == null) return "";
  const s = typeof v === "object" ? JSON.stringify(v) : String(v);
  return s.length > MAX_CELL_CHARS ? `${s.slice(0, MAX_CELL_CHARS)}…` : s;
}
