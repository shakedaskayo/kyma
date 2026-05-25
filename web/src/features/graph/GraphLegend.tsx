'use client';

import { getLabelColor } from '@/sdk/graph-layout';
import type { GraphStats } from '@/sdk/graph';
import { cn } from '@/lib/utils';

interface GraphLegendProps {
  stats: GraphStats | undefined;
  activeLabel: string | null;
  onLabelClick: (label: string | null) => void;
}

export function GraphLegend({ stats, activeLabel, onLabelClick }: GraphLegendProps) {
  if (!stats || Object.keys(stats.label_counts).length === 0) return null;

  const sorted = Object.entries(stats.label_counts).sort(
    ([, a], [, b]) => b - a,
  );

  return (
    <div className="pointer-events-auto rounded-md border border-border bg-background/90 p-2 text-xs shadow-sm backdrop-blur max-w-[200px]">
      <p className="mb-1.5 px-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        Labels
      </p>
      <div className="space-y-0.5">
        {sorted.map(([label, count]) => {
          const color = getLabelColor(label);
          const isActive = activeLabel === label;
          return (
            <button
              key={label}
              type="button"
              onClick={() => onLabelClick(label)}
              className={cn(
                'flex w-full items-center gap-2 rounded px-1.5 py-1 text-left transition-colors',
                isActive
                  ? 'bg-primary/15 text-foreground'
                  : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground',
              )}
            >
              <span
                className="h-2.5 w-2.5 shrink-0 rounded-full"
                style={{ backgroundColor: color }}
              />
              <span className="flex-1 truncate">{label}</span>
              <span className="shrink-0 font-mono text-[10px] tabular-nums">
                {count}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
