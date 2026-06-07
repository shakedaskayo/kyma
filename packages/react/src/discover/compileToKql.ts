// Compile the current Discover state (one or more sources + filter pills +
// optional time range) to a KQL string that can be pasted into the Query
// Editor as a starting point.
//
// • 0 sources → ""
// • 1 source  → single pipe (unchanged behaviour, no union wrapper)
// • 2+ sources from the same database → union withsource=_source (…), (…) | take 500

import type { Pill, SourceKey } from "./types";

export type ExportSource = {
  key: SourceKey;
  timestampColumn: string | null;
  stringColumns: string[];
};

export function compileToKql(
  sources: ExportSource | ExportSource[],
  pills: Pill[],
  timeRange?: { from: string; to: string } | null,
): string {
  const arr = Array.isArray(sources) ? sources : [sources];

  if (arr.length === 0) return "";

  if (arr.length === 1) {
    const s = arr[0];
    if (!s.key) return "";
    return onePipe(s.key, s.timestampColumn, s.stringColumns ?? [], pills, timeRange);
  }

  const branches = arr.map((s) => {
    const pipe = oneBranch(s.key, s.timestampColumn, s.stringColumns ?? [], pills, timeRange);
    return `(${pipe})`;
  });

  return `union withsource=_source ${branches.join(", ")} | take 500`;
}

function oneBranch(
  source: SourceKey,
  timestampColumn: string | null,
  stringColumns: string[],
  pills: Pill[],
  tr?: { from: string; to: string } | null,
): string {
  const dotIdx = source.indexOf(".");
  const tableExpr = dotIdx >= 0 ? source.slice(dotIdx + 1) : source;
  const clauses = pills.map((p) => pillToKql(p, stringColumns)).filter((s): s is string => s !== null);
  if (tr && timestampColumn !== null) {
    const col = timestampColumn;
    clauses.push(
      `${col} >= datetime("${tr.from}") and ${col} < datetime("${tr.to}")`,
    );
  }
  let out = tableExpr;
  for (const c of clauses) out += ` | where ${c}`;
  return out;
}

function onePipe(
  source: SourceKey,
  timestampColumn: string | null,
  stringColumns: string[],
  pills: Pill[],
  tr?: { from: string; to: string } | null,
): string {
  return oneBranch(source, timestampColumn, stringColumns, pills, tr) + " | take 500";
}

function pillToKql(p: Pill, stringColumns: string[]): string | null {
  const esc = (s: string) =>
    `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  switch (p.kind) {
    case "substring":
      if (stringColumns.length === 0) return null;
      return `(${stringColumns.map((c) => `${c} contains ${esc(p.value)}`).join(" or ")})`;
    case "eq":
      return `${p.field} == ${esc(p.value)}`;
    case "neq":
      return `${p.field} != ${esc(p.value)}`;
    case "exists":
      return `isnotnull(${p.field})`;
    case "cmp": {
      const op = { gt: ">", ge: ">=", lt: "<", le: "<=" }[p.op];
      return `${p.field} ${op} ${p.value}`;
    }
  }
}
