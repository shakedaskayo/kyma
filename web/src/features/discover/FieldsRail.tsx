import { useMemo } from "react";
import { Columns3, Filter } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { SourceState } from "./types";

type Props = {
  source: SourceState | null;
  columns: string[];
  onToggleColumn: (field: string) => void;
  onInsertFilter: (text: string) => void;
};

export function FieldsRail({ source, columns, onToggleColumn, onInsertFilter }: Props) {
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
      {fields.map((f) => {
        const isCol = columns.includes(f);
        return (
          <div
            key={f}
            className="flex items-center gap-1 px-2 py-1 rounded text-xs hover:bg-accent group"
          >
            <button
              type="button"
              className={`truncate flex-1 text-left font-mono ${isCol ? "text-primary" : ""}`}
              title={isCol ? `remove ${f} column from the stream` : `show ${f} as a column in the stream`}
              onClick={() => onToggleColumn(f)}
            >
              {f}
            </button>
            {isCol && <Columns3 className="size-3 text-primary" />}
            <Button
              variant="ghost"
              size="sm"
              className="opacity-0 group-hover:opacity-100 h-5 px-1"
              onClick={() => onInsertFilter(`${f}:*`)}
              aria-label={`filter to rows where ${f} exists`}
              title={`add ${f}:* to the query`}
            >
              <Filter className="size-3" />
            </Button>
          </div>
        );
      })}
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
