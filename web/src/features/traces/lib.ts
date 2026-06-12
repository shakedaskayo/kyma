// Pure helpers for the Traces surface: row types, span-tree assembly,
// duration formatting. No React.

export interface SpanRow {
  start_time: string;
  end_time: string;
  duration_ns: number;
  trace_id: string;
  span_id: string;
  parent_span_id: string | null;
  name: string;
  kind: string;
  status_code: string;
  status_message: string | null;
  service_name: string | null;
  subject: string | null;
  tenant: string | null;
  attributes_json: string;
}

export interface SpanNode {
  row: SpanRow;
  children: SpanNode[];
  depth: number;
}

/** Assemble parent/child trees from a flat span list. Orphans become roots. */
export function buildSpanTree(rows: SpanRow[]): SpanNode[] {
  const nodes = new Map<string, SpanNode>();
  for (const row of rows) nodes.set(row.span_id, { row, children: [], depth: 0 });
  const roots: SpanNode[] = [];
  for (const node of nodes.values()) {
    const pid = node.row.parent_span_id;
    const parent = pid ? nodes.get(pid) : undefined;
    if (parent) parent.children.push(node);
    else roots.push(node);
  }
  const sortRec = (list: SpanNode[], depth: number) => {
    list.sort((a, b) => a.row.start_time.localeCompare(b.row.start_time));
    for (const n of list) {
      n.depth = depth;
      sortRec(n.children, depth + 1);
    }
  };
  sortRec(roots, 0);
  return roots;
}

export function fmtDurationNs(ns: number): string {
  if (ns < 1_000) return `${ns}ns`;
  if (ns < 1_000_000) return `${+(ns / 1_000).toFixed(1)}µs`;
  if (ns < 1_000_000_000) return `${+(ns / 1_000_000).toFixed(1)}ms`;
  return `${+(ns / 1_000_000_000).toFixed(2)}s`;
}

/**
 * The query API returns timestamps without a timezone suffix but they are
 * UTC; `new Date()` would parse them as *local* time and shift every span by
 * the user's UTC offset. Tag them explicitly.
 */
export function utcIso(ts: string): string {
  return /(?:Z|[+-]\d{2}:\d{2})$/.test(ts) ? ts : `${ts}Z`;
}

/** Safe attributes_json → flat string map. */
export function parseAttrs(json: string): Record<string, string> {
  try {
    const obj = JSON.parse(json || "{}") as Record<string, unknown>;
    return Object.fromEntries(Object.entries(obj).map(([k, v]) => [k, String(v)]));
  } catch {
    return {};
  }
}

const HTTP_OP = /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS) (\S+)$/;

/** "GET /v1/query" → method + path; anything else passes through whole. */
export function splitOperation(name: string): { method: string | null; path: string } {
  const m = HTTP_OP.exec(name);
  return m ? { method: m[1], path: m[2] } : { method: null, path: name };
}
