import { useMemo, useState } from "react";
import { ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { buildSpanTree, fmtDurationNs, parseAttrs, type SpanNode } from "./lib";
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

// Depth → bar tint. Root stands out; nesting fades toward the background.
const DEPTH_BAR = [
  "bg-violet-400/90",
  "bg-violet-400/65",
  "bg-violet-400/45",
  "bg-violet-400/30",
];

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

  const root = flat[0]?.row;

  return (
    <div className="flex h-full flex-col">
      {root && (
        <div className="space-y-1 border-b border-border/50 px-3 py-2.5">
          <p className="truncate font-mono text-xs text-foreground/90" title={root.name}>
            {root.name}
          </p>
          <p className="flex items-center gap-3 text-[11px] text-muted-foreground">
            <span className="tabular-nums text-foreground/75">{fmtDurationNs(total * 1e6)}</span>
            <span>
              {rows.length} span{rows.length === 1 ? "" : "s"}
            </span>
            <span className="tabular-nums">{new Date(t0).toLocaleTimeString()}</span>
          </p>
        </div>
      )}

      {/* timeline ruler */}
      <div className="grid grid-cols-[minmax(0,11rem)_1fr] gap-x-2 border-b border-border/40 px-3 py-1">
        <span />
        <div className="relative h-4">
          {[0, 0.25, 0.5, 0.75, 1].map((f) => (
            <span
              key={f}
              className={cn(
                "absolute top-0 whitespace-nowrap text-[9px] tabular-nums text-muted-foreground/70",
                f === 1 && "-translate-x-full",
              )}
              style={{ left: `${f * 100}%` }}
            >
              {fmtDurationNs(total * f * 1e6)}
            </span>
          ))}
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-1 py-1">
        {flat.map((n) => (
          <WaterfallRow key={n.row.span_id} node={n} t0={t0} total={total} />
        ))}
      </div>
    </div>
  );
}

function WaterfallRow({ node, t0, total }: { node: SpanNode; t0: number; total: number }) {
  const r = node.row;
  const [open, setOpen] = useState(false);
  const start = new Date(r.start_time).getTime();
  const end = new Date(r.end_time).getTime();
  const leftPct = ((start - t0) / total) * 100;
  const widthPct = Math.max(0.8, ((end - start) / total) * 100);
  const error = r.status_code === "ERROR";
  const attrs = useMemo(() => Object.entries(parseAttrs(r.attributes_json)), [r.attributes_json]);
  // Put the duration label after the bar unless the bar nearly fills the
  // track — then inside it, right-aligned.
  const labelInside = leftPct + widthPct > 78;

  return (
    <div className="rounded-md transition-colors hover:bg-card/50">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="grid w-full grid-cols-[minmax(0,11rem)_1fr] items-center gap-x-2 px-2 py-1 text-left"
      >
        <span className="flex min-w-0 items-center gap-1" style={{ paddingLeft: `${node.depth * 12}px` }}>
          <ChevronRight
            className={cn(
              "h-3 w-3 shrink-0 text-muted-foreground/60 transition-transform",
              open && "rotate-90",
              attrs.length === 0 && "invisible",
            )}
          />
          <span
            className={cn(
              "h-1.5 w-1.5 shrink-0 rounded-full",
              error ? "bg-rose-400" : r.status_code === "OK" ? "bg-emerald-400" : "bg-muted-foreground/40",
            )}
          />
          <span
            className={cn("truncate font-mono text-[11px]", error ? "text-rose-200" : "text-foreground/85")}
            title={r.name}
          >
            {r.name}
          </span>
        </span>
        <span className="relative h-4">
          <span className="absolute inset-y-1.5 inset-x-0 rounded bg-border/20" />
          <span
            className={cn(
              "absolute inset-y-1 rounded",
              error ? "bg-rose-400/85" : DEPTH_BAR[Math.min(node.depth, DEPTH_BAR.length - 1)],
            )}
            style={{ left: `${leftPct}%`, width: `${widthPct}%` }}
          />
          <span
            className={cn(
              "absolute top-1/2 -translate-y-1/2 whitespace-nowrap text-[9px] tabular-nums",
              labelInside ? "text-background/90" : "text-muted-foreground",
            )}
            style={
              labelInside
                ? { right: `${Math.max(0, 100 - leftPct - widthPct) + 0.5}%` }
                : { left: `${Math.min(leftPct + widthPct + 1, 92)}%` }
            }
          >
            {fmtDurationNs(r.duration_ns)}
          </span>
        </span>
      </button>
      {open && attrs.length > 0 && (
        <dl className="mx-2 mb-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 rounded-md border border-border/40 bg-card/40 px-3 py-2 text-[11px]">
          {r.status_message && (
            <div className="contents">
              <dt className="font-mono text-rose-300/80">status</dt>
              <dd className="text-rose-200">{r.status_message}</dd>
            </div>
          )}
          {attrs.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="font-mono text-muted-foreground">{k}</dt>
              <dd className="truncate font-mono text-foreground/80" title={v}>
                {v}
              </dd>
            </div>
          ))}
        </dl>
      )}
    </div>
  );
}
