import { useMemo, useState, useRef, useCallback } from "react";
import { partitionColumns, formatCell } from "./columns";
import { pickMessageField, summarizeRow } from "./rowSummary";
import type { StreamRow } from "./stream";
import type { SourceState, SourceKey } from "./types";

const PAGE = 200;
const CHIP_COLORS = [
  "ky-border-blue-500/50", "ky-border-emerald-500/50", "ky-border-amber-500/50", "ky-border-red-500/50",
  "ky-border-violet-500/50", "ky-border-pink-500/50", "ky-border-teal-500/50", "ky-border-orange-500/50",
];

// ── Pure freeze-logic helpers (exported for unit tests) ─────────────────────

/**
 * Returns true when the scroll container is NOT at the top and new rows would
 * cause the view to jump — the user is reading history.
 */
export function shouldFreeze(scrollTop: number): boolean {
  return scrollTop > 0;
}

/**
 * The count of new rows that arrived while the view is frozen.
 * prevLen = row count when freeze started; nextLen = current row count.
 */
export function bufferedCount(prevLen: number, nextLen: number): number {
  return Math.max(0, nextLen - prevLen);
}

// ───────────────────────────────────────────────────────────────────────────

type Props = {
  rows: StreamRow[];
  sources: Map<SourceKey, SourceState>;
  columns: string[];
  onOpenRow: (source: SourceKey, row: Record<string, unknown>) => void;
};

export function StreamView({ rows, sources, columns, onOpenRow }: Props) {
  const [limit, setLimit] = useState(PAGE);
  const scrollRef = useRef<HTMLDivElement>(null);

  const [frozenRows, setFrozenRows] = useState<StreamRow[] | null>(null);
  const frozenLenRef = useRef<number>(0);

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (shouldFreeze(el.scrollTop)) {
      setFrozenRows((prev) => {
        if (prev === null) {
          frozenLenRef.current = rows.length;
          return rows;
        }
        return prev;
      });
    } else {
      setFrozenRows(null);
      frozenLenRef.current = 0;
    }
  }, [rows]);

  const jumpToTop = useCallback(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = 0;
    }
    setFrozenRows(null);
    frozenLenRef.current = 0;
  }, []);

  const displayRows = frozenRows ?? rows;
  const pendingCount = frozenRows !== null ? bufferedCount(frozenLenRef.current, rows.length) : 0;

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
    <div className="ky-relative ky-h-full ky-overflow-auto ky-text-xs ky-font-mono" ref={scrollRef} onScroll={handleScroll}>
      {pendingCount > 0 && (
        <div className="ky-sticky ky-top-0 ky-z-10 ky-flex ky-justify-center ky-pointer-events-none">
          <button
            type="button"
            className="ky-pointer-events-auto ky-mt-1 ky-px-3 ky-py-1 ky-rounded-full ky-bg-primary ky-text-primary-foreground ky-text-xs ky-shadow-md hover:ky-bg-primary/90"
            onClick={jumpToTop}
          >
            {pendingCount} new event{pendingCount === 1 ? "" : "s"} ↑
          </button>
        </div>
      )}

      {displayRows.slice(0, limit).map((r, i) => {
        const h = hints.get(r.source);
        const sum = summarizeRow(r.row, h?.message ?? null, h?.tsCol ?? null, [
          ...(h?.hiddenVectors ?? []),
          ...columns,
        ]);
        return (
          <div
            key={i}
            className="ky-flex ky-items-start ky-gap-2 ky-px-3 ky-py-1.5 ky-border-b ky-border-border/50 hover:ky-bg-accent ky-cursor-pointer"
            onClick={() => onOpenRow(r.source, r.row)}
          >
            <span className="ky-text-muted-foreground ky-tabular-nums ky-shrink-0 ky-w-[8ch]">{fmtTs(r.ts)}</span>
            <span
              className={`ky-shrink-0 ky-rounded ky-border ky-px-1 ky-text-[10px] ky-text-muted-foreground ${chipColor.get(r.source) ?? ""}`}
              title={r.source}
            >
              {r.source.split(".")[1] ?? r.source}
            </span>
            {columns.map((c) => (
              <span key={c} className="ky-shrink-0 ky-max-w-[24ch] ky-truncate" title={`${c}=${formatCell(r.row[c])}`}>
                {formatCell(r.row[c])}
              </span>
            ))}
            <span className="ky-min-w-0 ky-truncate">
              {sum.primary && <span>{sum.primary}</span>}
              {sum.rest.length > 0 && (
                <span className="ky-text-muted-foreground">
                  {sum.primary ? "  " : ""}
                  {sum.rest.map(([k, v]) => `${k}=${v}`).join(" ")}
                </span>
              )}
            </span>
          </div>
        );
      })}
      {displayRows.length > limit && (
        <button
          type="button"
          className="ky-w-full ky-p-2 ky-text-center ky-text-muted-foreground hover:ky-bg-accent"
          onClick={() => setLimit((l) => l + PAGE)}
        >
          show {Math.min(PAGE, displayRows.length - limit)} more of {displayRows.length.toLocaleString()} loaded
        </button>
      )}
    </div>
  );
}
