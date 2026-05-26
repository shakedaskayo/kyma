'use client';

import { cn } from '@/lib/utils';
import type { LayoutAlgorithm } from '@/sdk/graph-layout';

interface CanvasToolbarProps {
  layout: LayoutAlgorithm;
  onLayoutChange: (l: LayoutAlgorithm) => void;
}

const LAYOUTS: { value: LayoutAlgorithm; label: string }[] = [
  { value: 'tree', label: 'Tree' },
  { value: 'force', label: 'Force' },
  { value: 'radial', label: 'Radial' },
  { value: 'grid', label: 'Grid' },
];

export function CanvasToolbar({ layout, onLayoutChange }: CanvasToolbarProps) {
  return (
    <div className="pointer-events-auto flex items-center gap-1 rounded-md border border-border bg-background/90 p-1 text-xs shadow-sm backdrop-blur">
      {LAYOUTS.map(({ value, label }) => (
        <button
          key={value}
          type="button"
          onClick={() => onLayoutChange(value)}
          className={cn(
            'rounded px-2.5 py-1 transition-colors',
            layout === value
              ? 'bg-accent text-accent-foreground'
              : 'text-muted-foreground hover:bg-accent/60',
          )}
        >
          {label}
        </button>
      ))}
    </div>
  );
}
