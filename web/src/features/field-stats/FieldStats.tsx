import { useState, useMemo } from "react";
import { ChevronRight } from "lucide-react";
import type { Column } from "@/sdk/arrow";

const MAX_DISTINCT = 50;
const TOP_N        = 5;

type FieldStatsSectionProps = {
  col: Column;
  rows: Record<string, unknown>[];
  onAddFilter: (col: string, value: string) => void;
};

function FieldStatsSection({ col, rows, onAddFilter }: FieldStatsSectionProps) {
  const [open, setOpen] = useState(false);

  const { topValues, total, tooMany } = useMemo(() => {
    const counts = new Map<string, number>();
    let total = 0;
    for (const row of rows) {
      const v = row[col.name];
      if (v === null || v === undefined) continue;
      const s = String(v);
      counts.set(s, (counts.get(s) ?? 0) + 1);
      total++;
    }
    if (counts.size > MAX_DISTINCT) return { topValues: [], total, tooMany: true };

    const sorted = Array.from(counts.entries())
      .sort((a, b) => b[1] - a[1])
      .slice(0, TOP_N);

    return { topValues: sorted, total, tooMany: false };
  }, [col, rows]);

  return (
    <div className="border-b border-muted/40 last:border-0">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-1 px-3 py-1.5 text-left text-xs hover:bg-accent/30 transition-colors"
      >
        <ChevronRight
          className="h-3 w-3 shrink-0 text-muted-foreground transition-transform"
          style={{ transform: open ? "rotate(90deg)" : "rotate(0deg)" }}
        />
        <span className="font-medium truncate flex-1">{col.name}</span>
        <span className="ml-auto shrink-0 text-[10px] text-muted-foreground">{col.kind}</span>
      </button>
      {open && (
        <div className="px-3 pb-2">
          {tooMany ? (
            <p className="text-[10px] text-muted-foreground italic">{/* distinct values > 50 */}many unique values</p>
          ) : (
            <div className="space-y-0.5">
              {topValues.map(([val, count]) => {
                const pct = total > 0 ? (count / total) * 100 : 0;
                return (
                  <button
                    key={val}
                    onClick={() => onAddFilter(col.name, val)}
                    className="group flex w-full flex-col gap-0.5 rounded px-1 py-0.5 text-left hover:bg-accent/40 transition-colors"
                    title={`Filter: ${col.name} == "${val}"`}
                  >
                    <div className="flex items-center justify-between">
                      <span className="max-w-[140px] truncate font-mono text-[10px] text-foreground group-hover:text-primary">
                        {val === "" ? <span className="italic text-muted-foreground">empty</span> : val}
                      </span>
                      <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">{count.toLocaleString()}</span>
                    </div>
                    <div className="h-[2px] w-full rounded-full bg-muted/60">
                      <div
                        className="h-full rounded-full bg-primary/60"
                        style={{ width: `${pct}%` }}
                      />
                    </div>
                  </button>
                );
              })}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export type FieldStatsProps = {
  rows: Record<string, unknown>[];
  columns: Column[];
  onAddFilter: (col: string, value: string) => void;
};

export function FieldStats({ rows, columns, onAddFilter }: FieldStatsProps) {
  // Only show non-time columns (time columns don't benefit from value counts)
  const nonTimeCols = useMemo(
    () => columns.filter((c) => c.kind !== "time"),
    [columns],
  );

  if (nonTimeCols.length === 0) return null;

  return (
    <div className="overflow-y-auto">
      <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-b border-muted/40">
        Field Values
      </div>
      {nonTimeCols.map((col) => (
        <FieldStatsSection
          key={col.name}
          col={col}
          rows={rows}
          onAddFilter={onAddFilter}
        />
      ))}
    </div>
  );
}
