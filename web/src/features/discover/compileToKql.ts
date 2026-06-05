// Compile the current Discover state (single source + filter pills + optional
// time range) to a KQL string that can be pasted into the Query Editor as a
// starting point. Single source only — the kyma-kql engine has no union
// support yet; union will be added here once the engine supports it.

import type { Pill, SourceKey } from "./types";

export function compileToKql(
  source: { key: SourceKey; timestampColumn: string | null; stringColumns?: string[] },
  pills: Pill[],
  timeRange?: { from: string; to: string } | null,
): string {
  if (!source.key) return "";
  return onePipe(source.key, source.timestampColumn, source.stringColumns ?? [], pills, timeRange);
}

function onePipe(
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
  out += " | take 500";
  return out;
}

function pillToKql(p: Pill, stringColumns: string[]): string | null {
  const esc = (s: string) =>
    `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  switch (p.kind) {
    case "substring":
      // The kyma-kql engine expands substring to a disjunction over schema
      // STRING columns. Mirror that here using the sampled string columns.
      // If none are available, drop the pill (best-effort; user can refine).
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
