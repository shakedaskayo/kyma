import { useMemo } from "react";
import { cn } from "@/lib/utils";
import { buildSpanTree, fmtDurationNs, STATUS_TONE, type SpanNode } from "./lib";
import { useTraceSpans } from "./useTraces";

/** Flatten the tree depth-first for row rendering. */
function flatten(nodes: SpanNode[]): SpanNode[] {
  const out: SpanNode[] = [];
  const walk = (n: SpanNode) => {
    out.push(n);
    n.children.forEach(walk);
  };
  nodes.forEach(walk);
  return out;
}

export function TraceWaterfall({ traceId }: { traceId: string }) {
  const { rows, loading } = useTraceSpans(traceId);
  const { flat, t0, total } = useMemo(() => {
    const tree = buildSpanTree(rows);
    const flat = flatten(tree);
    const starts = rows.map((r) => new Date(r.start_time).getTime());
    const ends = rows.map((r) => new Date(r.end_time).getTime());
    const t0 = Math.min(...(starts.length ? starts : [0]));
    const total = Math.max(1, Math.max(...(ends.length ? ends : [1])) - t0);
    return { flat, t0, total };
  }, [rows]);

  if (loading) return <p className="p-4 text-xs text-muted-foreground">Loading spans…</p>;
  if (rows.length === 0)
    return <p className="p-4 text-xs text-muted-foreground">No spans for this trace.</p>;

  return (
    <div className="space-y-1 p-3">
      {flat.map((n) => (
        <WaterfallRow key={n.row.span_id} node={n} t0={t0} total={total} />
      ))}
    </div>
  );
}

function WaterfallRow({ node, t0, total }: { node: SpanNode; t0: number; total: number }) {
  const r = node.row;
  const start = new Date(r.start_time).getTime();
  const end = new Date(r.end_time).getTime();
  const leftPct = ((start - t0) / total) * 100;
  const widthPct = Math.max(0.5, ((end - start) / total) * 100);
  const attrs = useMemo(() => {
    try {
      return Object.entries(JSON.parse(r.attributes_json || "{}") as Record<string, unknown>);
    } catch {
      return [];
    }
  }, [r.attributes_json]);

  return (
    <details className="group rounded-md border border-border/40 bg-card/30 hover:border-border/60">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2 py-1.5">
        <span
          className="truncate text-xs text-foreground/85"
          style={{ paddingLeft: `${node.depth * 14}px` }}
          title={r.name}
        >
          {r.name}
        </span>
        <span className={cn("text-xs", STATUS_TONE[r.status_code] ?? STATUS_TONE["UNSET"])}>
          {r.status_code}
        </span>
        <span className="ml-auto shrink-0 text-xs tabular-nums text-muted-foreground">
          {fmtDurationNs(r.duration_ns)}
        </span>
        <span className="relative h-1.5 w-40 shrink-0 overflow-hidden rounded bg-border/30">
          <span
            className={cn(
              "absolute inset-y-0 rounded",
              r.status_code === "ERROR" ? "bg-rose-400/80" : "bg-violet-400/80",
            )}
            style={{ left: `${leftPct}%`, width: `${widthPct}%` }}
          />
        </span>
      </summary>
      {attrs.length > 0 && (
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 border-t border-border/40 px-3 py-2 text-xs">
          {attrs.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="font-mono text-muted-foreground">{k}</dt>
              <dd className="truncate text-foreground/80" title={String(v)}>
                {String(v)}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </details>
  );
}
