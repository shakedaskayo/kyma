// Compile the current Discover state (scope sources + filter pills + optional
// time range) to a KQL string that can be pasted into the Query Editor as
// a starting point. Multi-source → union of per-source pipes. Single source →
// a single pipe. Mirrors the engine's per-source compilation rules at a
// best-effort level; the editor user can clean up dropped clauses.

import type { Pill, SourceKey } from "./types";

export function compileToKql(
  sources: SourceKey[],
  pills: Pill[],
  timeRange?: { from: string; to: string } | null,
): string {
  if (sources.length === 0) return "";
  const pipes = sources.map((s) => onePipe(s, pills, timeRange));
  if (pipes.length === 1) return pipes[0]!;
  return "union\n  " + pipes.join(",\n  ");
}

function onePipe(
  source: SourceKey,
  pills: Pill[],
  tr?: { from: string; to: string } | null,
): string {
  const dotIdx = source.indexOf(".");
  const tableExpr = dotIdx >= 0 ? source.slice(dotIdx + 1) : source;
  const clauses = pills.map(pillToKql).filter((s): s is string => s !== null);
  if (tr) {
    clauses.push(
      `timestamp >= datetime("${tr.from}") and timestamp < datetime("${tr.to}")`,
    );
  }
  let out = tableExpr;
  for (const c of clauses) out += ` | where ${c}`;
  out += " | take 500";
  return out;
}

function pillToKql(p: Pill): string | null {
  const esc = (s: string) =>
    `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  switch (p.kind) {
    case "substring":
      return `* contains ${esc(p.value)}`;
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
