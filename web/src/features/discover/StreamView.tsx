import { useMemo, useState, useRef, useCallback, useEffect } from "react";
import { partitionColumns, formatCell } from "./columns";
import { pickMessageField, summarizeRow } from "./rowSummary";
import type { StreamRow } from "./stream";
import type { SourceState, SourceKey } from "./types";

const PAGE = 200;
const CHIP_COLORS = [
  "border-blue-500/50", "border-emerald-500/50", "border-amber-500/50", "border-red-500/50",
  "border-violet-500/50", "border-pink-500/50", "border-teal-500/50", "border-orange-500/50",
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

  // Frozen snapshot: when the user scrolls away from the top, we keep
  // rendering the rows we had when freeze started. New rows accumulate in
  // `rows` but are not shown until the user jumps back to the top.
  const [frozenRows, setFrozenRows] = useState<StreamRow[] | null>(null);
  const frozenLenRef = useRef<number>(0);

  // Detect scroll position and freeze/unfreeze
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (shouldFreeze(el.scrollTop)) {
      // Freeze: snapshot the current rows if not already frozen
      setFrozenRows((prev) => {
        if (prev === null) {
          frozenLenRef.current = rows.length;
          return rows;
        }
        return prev;
      });
    } else {
      // Unfreeze: back at top
      setFrozenRows(null);
      frozenLenRef.current = 0;
    }
  }, [rows]);

  // Keep frozen snapshot's length in sync when rows prop changes while frozen
  // (needed so bufferedCount works correctly)
  const prevRowsLenRef = useRef(rows.length);
  useEffect(() => {
    if (frozenRows !== null && rows.length !== prevRowsLenRef.current) {
      // rows changed while frozen — frozenLenRef already captured the len at freeze time
    }
    prevRowsLenRef.current = rows.length;
  }, [rows.length, frozenRows]);

  const jumpToTop = useCallback(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = 0;
    }
    setFrozenRows(null);
    frozenLenRef.current = 0;
  }, []);

  const displayRows = frozenRows ?? rows;
  const pendingCount = frozenRows !== null ? bufferedCount(frozenLenRef.current, rows.length) : 0;

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
    <div className="relative text-xs font-mono" ref={scrollRef} onScroll={handleScroll}>
      {/* Sticky "new events" pill shown while frozen and new rows have arrived */}
      {pendingCount > 0 && (
        <div className="sticky top-0 z-10 flex justify-center pointer-events-none">
          <button
            type="button"
            className="pointer-events-auto mt-1 px-3 py-1 rounded-full bg-primary text-primary-foreground text-xs shadow-md hover:bg-primary/90"
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
      {displayRows.length > limit && (
        <button
          type="button"
          className="w-full p-2 text-center text-muted-foreground hover:bg-accent"
          onClick={() => setLimit((l) => l + PAGE)}
        >
          show {Math.min(PAGE, displayRows.length - limit)} more of {displayRows.length.toLocaleString()} loaded
        </button>
      )}
    </div>
  );
}
