'use client';

import { Eye, EyeOff } from 'lucide-react';
import { getLabelColor, getRelationshipColor } from '@/sdk/graph-layout';
import type { GraphStats } from '@/sdk/graph';
import { cn } from '@/lib/utils';

interface GraphLegendProps {
  stats: GraphStats | undefined;
  /** Node-type labels currently hidden from the canvas. */
  hiddenLabels: string[];
  onToggleLabel: (label: string) => void;
  activeRel: string | null;
  onRelClick: (rel: string) => void;
}

export function GraphLegend({ stats, hiddenLabels, onToggleLabel, activeRel, onRelClick }: GraphLegendProps) {
  if (!stats) return null;
  const labels = Object.entries(stats.label_counts).sort(([, a], [, b]) => b - a);
  const rels = Object.entries(stats.relationship_type_counts).sort(([, a], [, b]) => b - a);
  if (labels.length === 0 && rels.length === 0) return null;

  return (
    <div className="pointer-events-auto max-w-[220px] space-y-2.5 rounded-md border border-border bg-background/90 p-2 text-xs shadow-sm backdrop-blur">
      {/* Node types — click to show/hide that type on the canvas */}
      {labels.length > 0 && (
        <div>
          <p className="mb-1 px-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Node types
          </p>
          <div className="space-y-0.5">
            {labels.map(([label, count]) => {
              const color = getLabelColor(label);
              const hidden = hiddenLabels.includes(label);
              return (
                <button
                  key={label}
                  type="button"
                  onClick={() => onToggleLabel(label)}
                  title={hidden ? `Show ${label}` : `Hide ${label}`}
                  className={cn(
                    'group flex w-full items-center gap-2 rounded px-1.5 py-1 text-left transition-colors hover:bg-muted/50',
                    hidden ? 'text-muted-foreground/50' : 'text-muted-foreground hover:text-foreground',
                  )}
                >
                  <span
                    className="h-2.5 w-2.5 shrink-0 rounded-full"
                    style={{ backgroundColor: color, opacity: hidden ? 0.3 : 1 }}
                  />
                  <span className={cn('flex-1 truncate', hidden && 'line-through')}>{label}</span>
                  <span className="shrink-0 font-mono text-[10px] tabular-nums">{count}</span>
                  {hidden ? (
                    <EyeOff className="h-3 w-3 shrink-0" />
                  ) : (
                    <Eye className="h-3 w-3 shrink-0 opacity-0 group-hover:opacity-60" />
                  )}
                </button>
              );
            })}
          </div>
        </div>
      )}

      {/* Relationships — click to focus a single relationship type */}
      {rels.length > 0 && (
        <div>
          <p className="mb-1 px-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Relationships
          </p>
          <div className="space-y-0.5">
            {rels.map(([rel, count]) => {
              const color = getRelationshipColor(rel);
              const isActive = activeRel === rel;
              return (
                <button
                  key={rel}
                  type="button"
                  onClick={() => onRelClick(rel)}
                  className={cn(
                    'flex w-full items-center gap-2 rounded px-1.5 py-1 text-left transition-colors',
                    isActive
                      ? 'bg-primary/15 text-foreground'
                      : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground',
                  )}
                >
                  <span className="h-[3px] w-3.5 shrink-0 rounded-full" style={{ backgroundColor: color }} />
                  <span className="flex-1 truncate">{rel}</span>
                  <span className="shrink-0 font-mono text-[10px] tabular-nums">{count}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}
      {activeRel && (
        <button
          type="button"
          onClick={() => onRelClick(activeRel)}
          className="w-full rounded px-1.5 py-1 text-center text-[10px] text-muted-foreground hover:bg-muted/50 hover:text-foreground"
        >
          Clear relationship filter
        </button>
      )}
    </div>
  );
}
