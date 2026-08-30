import { useState, useMemo } from "react";
import { ChevronRight } from "lucide-react";
import type { Column } from "@pensieve-ai/client";

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
    <div className="ky-border-b ky-border-muted/40 last:ky-border-0">
      <button
        onClick={() => setOpen((o) => !o)}
        className="ky-flex ky-w-full ky-items-center ky-gap-1 ky-px-3 ky-py-1.5 ky-text-left ky-text-xs hover:ky-bg-accent/30 ky-transition-colors"
      >
        <ChevronRight
          className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground ky-transition-transform"
          style={{ transform: open ? "rotate(90deg)" : "rotate(0deg)" }}
        />
        <span className="ky-font-medium ky-truncate ky-flex-1">{col.name}</span>
        <span className="ky-ml-auto ky-shrink-0 ky-text-[10px] ky-text-muted-foreground">{col.kind}</span>
      </button>
      {open && (
        <div className="ky-px-3 ky-pb-2">
          {tooMany ? (
            <p className="ky-text-[10px] ky-text-muted-foreground ky-italic">many unique values</p>
          ) : (
            <div className="ky-space-y-0.5">
              {topValues.map(([val, count]) => {
                const pct = total > 0 ? (count / total) * 100 : 0;
                return (
                  <button
                    key={val}
                    onClick={() => onAddFilter(col.name, val)}
                    className="ky-group ky-flex ky-w-full ky-flex-col ky-gap-0.5 ky-rounded ky-px-1 ky-py-0.5 ky-text-left hover:ky-bg-accent/40 ky-transition-colors"
                    title={`Filter: ${col.name} == "${val}"`}
                  >
                    <div className="ky-flex ky-items-center ky-justify-between">
                      <span className="ky-max-w-[140px] ky-truncate ky-font-mono ky-text-[10px] ky-text-foreground group-hover:ky-text-primary">
                        {val === "" ? <span className="ky-italic ky-text-muted-foreground">empty</span> : val}
                      </span>
                      <span className="ky-shrink-0 ky-text-[10px] ky-tabular-nums ky-text-muted-foreground">{count.toLocaleString()}</span>
                    </div>
                    <div className="ky-h-[2px] ky-w-full ky-rounded-full ky-bg-muted/60">
                      <div
                        className="ky-h-full ky-rounded-full ky-bg-primary/60"
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
  const nonTimeCols = useMemo(
    () => columns.filter((c) => c.kind !== "time"),
    [columns],
  );

  if (nonTimeCols.length === 0) return null;

  return (
    <div className="ky-overflow-y-auto">
      <div className="ky-px-3 ky-py-1.5 ky-text-[10px] ky-font-semibold ky-uppercase ky-tracking-wider ky-text-muted-foreground ky-border-b ky-border-muted/40">
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
