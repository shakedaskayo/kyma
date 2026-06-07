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
      <div className="ky-text-xs ky-text-muted-foreground ky-p-2">
        Select a source to see its fields.
      </div>
    );
  }
  if (fields.length === 0) {
    return (
      <div className="ky-text-xs ky-text-muted-foreground ky-p-2">
        {source.progress === "done" ? "No rows in this source." : "Loading…"}
      </div>
    );
  }
  return (
    <div className="ky-space-y-0.5">
      <div className="ky-text-xs ky-font-medium ky-text-muted-foreground ky-px-2 ky-py-1 ky-uppercase ky-tracking-wide">
        Fields in {source.source}
      </div>
      {fields.map((f) => {
        const isCol = columns.includes(f);
        return (
          <div
            key={f}
            className="ky-flex ky-items-center ky-gap-1 ky-px-2 ky-py-1 ky-rounded ky-text-xs hover:ky-bg-accent ky-group"
          >
            <button
              type="button"
              className={`ky-truncate ky-flex-1 ky-text-left ky-font-mono ${isCol ? "ky-text-primary" : ""}`}
              title={isCol ? `remove ${f} column from the stream` : `show ${f} as a column in the stream`}
              onClick={() => onToggleColumn(f)}
            >
              {f}
            </button>
            {isCol && <Columns3 className="ky-size-3 ky-text-primary" />}
            <Button
              variant="ghost"
              size="sm"
              className="ky-opacity-0 group-hover:ky-opacity-100 ky-h-5 ky-px-1"
              onClick={() => onInsertFilter(`${f}:*`)}
              aria-label={`filter to rows where ${f} exists`}
              title={`add ${f}:* to the query`}
            >
              <Filter className="ky-size-3" />
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
