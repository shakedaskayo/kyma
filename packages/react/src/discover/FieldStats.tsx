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
    <div className="pv-border-b pv-border-muted/40 last:pv-border-0">
      <button
        onClick={() => setOpen((o) => !o)}
        className="pv-flex pv-w-full pv-items-center pv-gap-1 pv-px-3 pv-py-1.5 pv-text-left pv-text-xs hover:pv-bg-accent/30 pv-transition-colors"
      >
        <ChevronRight
          className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground pv-transition-transform"
          style={{ transform: open ? "rotate(90deg)" : "rotate(0deg)" }}
        />
        <span className="pv-font-medium pv-truncate pv-flex-1">{col.name}</span>
        <span className="pv-ml-auto pv-shrink-0 pv-text-[10px] pv-text-muted-foreground">{col.kind}</span>
      </button>
      {open && (
        <div className="pv-px-3 pv-pb-2">
          {tooMany ? (
            <p className="pv-text-[10px] pv-text-muted-foreground pv-italic">many unique values</p>
          ) : (
            <div className="pv-space-y-0.5">
              {topValues.map(([val, count]) => {
                const pct = total > 0 ? (count / total) * 100 : 0;
                return (
                  <button
                    key={val}
                    onClick={() => onAddFilter(col.name, val)}
                    className="pv-group pv-flex pv-w-full pv-flex-col pv-gap-0.5 pv-rounded pv-px-1 pv-py-0.5 pv-text-left hover:pv-bg-accent/40 pv-transition-colors"
                    title={`Filter: ${col.name} == "${val}"`}
                  >
                    <div className="pv-flex pv-items-center pv-justify-between">
                      <span className="pv-max-w-[140px] pv-truncate pv-font-mono pv-text-[10px] pv-text-foreground group-hover:pv-text-primary">
                        {val === "" ? <span className="pv-italic pv-text-muted-foreground">empty</span> : val}
                      </span>
                      <span className="pv-shrink-0 pv-text-[10px] pv-tabular-nums pv-text-muted-foreground">{count.toLocaleString()}</span>
                    </div>
                    <div className="pv-h-[2px] pv-w-full pv-rounded-full pv-bg-muted/60">
                      <div
                        className="pv-h-full pv-rounded-full pv-bg-primary/60"
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
    <div className="pv-overflow-y-auto">
      <div className="pv-px-3 pv-py-1.5 pv-text-[10px] pv-font-semibold pv-uppercase pv-tracking-wider pv-text-muted-foreground pv-border-b pv-border-muted/40">
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
