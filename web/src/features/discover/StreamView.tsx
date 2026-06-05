import { useMemo, useState } from "react";
import { partitionColumns, formatCell } from "./columns";
import { pickMessageField, summarizeRow } from "./rowSummary";
import type { StreamRow } from "./stream";
import type { SourceState, SourceKey } from "./types";

const PAGE = 200;
const CHIP_COLORS = [
  "border-blue-500/50", "border-emerald-500/50", "border-amber-500/50", "border-red-500/50",
  "border-violet-500/50", "border-pink-500/50", "border-teal-500/50", "border-orange-500/50",
];

type Props = {
  rows: StreamRow[];
  sources: Map<SourceKey, SourceState>;
  columns: string[];
  onOpenRow: (source: SourceKey, row: Record<string, unknown>) => void;
};

export function StreamView({ rows, sources, columns, onOpenRow }: Props) {
  const [limit, setLimit] = useState(PAGE);

  // Per-source presentation hints, computed once per result set.
  const hints = useMemo(() => {
    const m = new Map<SourceKey, { message: string | null; hiddenVectors: string[]; tsCol: string | null }>();
    for (const [key, s] of sources) {
      const { hiddenVectors } = partitionColumns(s.rows);
      m.set(key, { message: pickMessageField(s.rows), hiddenVectors, tsCol: s.timestampColumn });
    }
    return m;
  }, [sources]);

  const chipColor = useMemo(() => {
    const m = new Map<SourceKey, string>();
    Array.from(sources.keys()).forEach((k, i) => m.set(k, CHIP_COLORS[i % CHIP_COLORS.length]));
    return m;
  }, [sources]);

  const fmtTs = (ts: number | null) =>
    ts == null ? "—" : new Date(ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });

  return (
    <div className="text-xs font-mono">
      {rows.slice(0, limit).map((r, i) => {
        const h = hints.get(r.source);
        const sum = summarizeRow(r.row, h?.message ?? null, h?.tsCol ?? null, [
          ...(h?.hiddenVectors ?? []),
          ...columns,
        ]);
        return (
          <div
            key={i}
            className="flex items-start gap-2 px-3 py-1.5 border-b border-border/50 hover:bg-accent cursor-pointer"
            onClick={() => onOpenRow(r.source, r.row)}
          >
            <span className="text-muted-foreground tabular-nums shrink-0 w-[8ch]">{fmtTs(r.ts)}</span>
            <span
              className={`shrink-0 rounded border px-1 text-[10px] text-muted-foreground ${chipColor.get(r.source) ?? ""}`}
              title={r.source}
            >
              {r.source.split(".")[1] ?? r.source}
            </span>
            {columns.map((c) => (
              <span key={c} className="shrink-0 max-w-[24ch] truncate" title={`${c}=${formatCell(r.row[c])}`}>
                {formatCell(r.row[c])}
              </span>
            ))}
            <span className="min-w-0 truncate">
              {sum.primary && <span>{sum.primary}</span>}
              {sum.rest.length > 0 && (
                <span className="text-muted-foreground">
                  {sum.primary ? "  " : ""}
                  {sum.rest.map(([k, v]) => `${k}=${v}`).join(" ")}
                </span>
              )}
            </span>
          </div>
        );
      })}
      {rows.length > limit && (
        <button
          type="button"
          className="w-full p-2 text-center text-muted-foreground hover:bg-accent"
          onClick={() => setLimit((l) => l + PAGE)}
        >
          show {Math.min(PAGE, rows.length - limit)} more of {rows.length.toLocaleString()} loaded
        </button>
      )}
    </div>
  );
}
