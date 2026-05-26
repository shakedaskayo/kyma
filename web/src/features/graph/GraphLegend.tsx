'use client';

import { getLabelColor, getRelationshipColor } from '@/sdk/graph-layout';
import type { GraphStats } from '@/sdk/graph';
import { cn } from '@/lib/utils';

interface GraphLegendProps {
  stats: GraphStats | undefined;
  activeLabel: string | null;
  onLabelClick: (label: string) => void;
  activeRel: string | null;
  onRelClick: (rel: string) => void;
}

function Section({
  title,
  entries,
  colorOf,
  shape,
  active,
  onClick,
}: {
  title: string;
  entries: [string, number][];
  colorOf: (k: string) => string;
  shape: 'dot' | 'bar';
  active: string | null;
  onClick: (k: string) => void;
}) {
  if (entries.length === 0) return null;
  return (
    <div>
      <p className="mb-1 px-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </p>
      <div className="space-y-0.5">
        {entries.map(([key, count]) => {
          const color = colorOf(key);
          const isActive = active === key;
          return (
            <button
              key={key}
              type="button"
              onClick={() => onClick(key)}
              className={cn(
                'flex w-full items-center gap-2 rounded px-1.5 py-1 text-left transition-colors',
                isActive
                  ? 'bg-primary/15 text-foreground'
                  : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground',
              )}
            >
              {shape === 'dot' ? (
                <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ backgroundColor: color }} />
              ) : (
                <span className="h-[3px] w-3.5 shrink-0 rounded-full" style={{ backgroundColor: color }} />
              )}
              <span className="flex-1 truncate">{key}</span>
              <span className="shrink-0 font-mono text-[10px] tabular-nums">{count}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function GraphLegend({ stats, activeLabel, onLabelClick, activeRel, onRelClick }: GraphLegendProps) {
  if (!stats) return null;
  const labels = Object.entries(stats.label_counts).sort(([, a], [, b]) => b - a);
  const rels = Object.entries(stats.relationship_type_counts).sort(([, a], [, b]) => b - a);
  if (labels.length === 0 && rels.length === 0) return null;

  return (
    <div className="pointer-events-auto max-w-[210px] space-y-2.5 rounded-md border border-border bg-background/90 p-2 text-xs shadow-sm backdrop-blur">
      <Section
        title="Node types"
        entries={labels}
        colorOf={getLabelColor}
        shape="dot"
        active={activeLabel}
        onClick={onLabelClick}
      />
      <Section
        title="Relationships"
        entries={rels}
        colorOf={getRelationshipColor}
        shape="bar"
        active={activeRel}
        onClick={onRelClick}
      />
      {(activeLabel || activeRel) && (
        <button
          type="button"
          onClick={() => {
            if (activeLabel) onLabelClick(activeLabel);
            if (activeRel) onRelClick(activeRel);
          }}
          className="w-full rounded px-1.5 py-1 text-center text-[10px] text-muted-foreground hover:bg-muted/50 hover:text-foreground"
        >
          Clear filter
        </button>
      )}
    </div>
  );
}
