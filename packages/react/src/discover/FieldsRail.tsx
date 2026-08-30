import { useMemo } from "react";
import { Columns3, Filter } from "lucide-react";
import { Button } from "../internal/ui/button";
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
      <div className="pv-text-xs pv-text-muted-foreground pv-p-2">
        Select a source to see its fields.
      </div>
    );
  }
  if (fields.length === 0) {
    return (
      <div className="pv-text-xs pv-text-muted-foreground pv-p-2">
        {source.progress === "done" ? "No rows in this source." : "Loading…"}
      </div>
    );
  }
  return (
    <div className="pv-space-y-0.5">
      <div className="pv-text-xs pv-font-medium pv-text-muted-foreground pv-px-2 pv-py-1 pv-uppercase pv-tracking-wide">
        Fields in {source.source}
      </div>
      {fields.map((f) => {
        const isCol = columns.includes(f);
        return (
          <div
            key={f}
            className="pv-flex pv-items-center pv-gap-1 pv-px-2 pv-py-1 pv-rounded pv-text-xs hover:pv-bg-accent pv-group"
          >
            <button
              type="button"
              className={`pv-truncate pv-flex-1 pv-text-left pv-font-mono ${isCol ? "pv-text-primary" : ""}`}
              title={isCol ? `remove ${f} column from the stream` : `show ${f} as a column in the stream`}
              onClick={() => onToggleColumn(f)}
            >
              {f}
            </button>
            {isCol && <Columns3 className="pv-size-3 pv-text-primary" />}
            <Button
              variant="ghost"
              size="sm"
              className="pv-opacity-0 group-hover:pv-opacity-100 pv-h-5 pv-px-1"
              onClick={() => onInsertFilter(`${f}:*`)}
              aria-label={`filter to rows where ${f} exists`}
              title={`add ${f}:* to the query`}
            >
              <Filter className="pv-size-3" />
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
