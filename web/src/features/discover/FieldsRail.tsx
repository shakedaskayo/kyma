import { useMemo } from "react";
import { Button } from "@/components/ui/button";
import type { Pill, SourceState } from "./types";

type Props = {
  source: SourceState | null;
  onAddPill: (p: Pill) => void;
};

export function FieldsRail({ source, onAddPill }: Props) {
  const fields = useMemo(() => extractFields(source), [source]);
  if (!source) {
    return (
      <div className="text-xs text-muted-foreground p-2">
        Select a source to see its fields.
      </div>
    );
  }
  if (fields.length === 0) {
    return (
      <div className="text-xs text-muted-foreground p-2">
        {source.progress === "done" ? "No rows in this source." : "Loading…"}
      </div>
    );
  }
  return (
    <div className="space-y-0.5">
      <div className="text-xs font-medium text-muted-foreground px-2 py-1 uppercase tracking-wide">
        Fields in {source.source}
      </div>
      {fields.map((f) => (
        <div
          key={f}
          className="flex items-center gap-1 px-2 py-1 rounded text-xs hover:bg-accent group"
        >
          <span className="truncate flex-1 font-mono" title={f}>{f}</span>
          <Button
            variant="ghost"
            size="sm"
            className="opacity-0 group-hover:opacity-100 h-5 px-1 text-[10px]"
            onClick={() => onAddPill({ kind: "exists", field: f })}
            aria-label={`add exists filter for ${f}`}
          >
            +exists
          </Button>
        </div>
      ))}
    </div>
  );
}

function extractFields(s: SourceState | null): string[] {
  if (!s) return [];
  const seen = new Set<string>();
  for (const row of s.rows.slice(0, 50)) {
    for (const k of Object.keys(row)) seen.add(k);
  }
  return Array.from(seen).sort();
}
