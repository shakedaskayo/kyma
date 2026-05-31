import { useMemo, useState } from "react";
import { ChevronRight, ChevronDown, AlertCircle } from "lucide-react";
import type { SourceState } from "./types";

type Props = {
  src: SourceState;
  onOpenRow: (row: Record<string, unknown>) => void;
};

export function SourceSection({ src, onOpenRow }: Props) {
  const [open, setOpen] = useState(true);
  const cols = useMemo(() => columnsFor(src), [src]);

  return (
    <div className="border-b">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 w-full text-left px-3 py-2 hover:bg-accent"
      >
        {open ? <ChevronDown className="size-4" /> : <ChevronRight className="size-4" />}
        <span className="font-mono font-medium text-sm">{src.source}</span>
        <span className="text-xs text-muted-foreground tabular-nums">
          {src.total.toLocaleString()} hits{src.capped ? " (capped)" : ""}
        </span>
        {src.error && (
          <span className="ml-auto inline-flex items-center gap-1 text-xs text-destructive">
            <AlertCircle className="size-3" /> {src.error.code}
          </span>
        )}
      </button>
      {open && (
        <div className="overflow-auto max-h-96 border-t">
          {src.error ? (
            <div className="p-3 text-sm text-destructive font-mono whitespace-pre-wrap">
              {src.error.message}
            </div>
          ) : src.rows.length === 0 ? (
            <div className="p-3 text-sm text-muted-foreground">
              {src.progress === "done" ? "no rows" : "loading…"}
            </div>
          ) : (
            <table className="w-full text-xs font-mono">
              <thead className="sticky top-0 bg-background z-10">
                <tr>
                  {cols.map((c) => (
                    <th
                      key={c}
                      className="text-left px-2 py-1 border-b font-medium"
                    >
                      {c}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {src.rows.slice(0, 200).map((r, i) => (
                  <tr
                    key={i}
                    className="hover:bg-accent cursor-pointer"
                    onClick={() => onOpenRow(r)}
                  >
                    {cols.map((c) => (
                      <td
                        key={c}
                        className="px-2 py-1 truncate max-w-xs align-top"
                        title={String(r[c] ?? "")}
                      >
                        {format(r[c])}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {src.rows.length > 200 && (
            <div className="p-2 text-xs text-muted-foreground">
              showing first 200 of {src.rows.length} loaded rows
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function columnsFor(src: SourceState): string[] {
  const seen = new Set<string>();
  for (const r of src.rows.slice(0, 50)) {
    for (const k of Object.keys(r)) seen.add(k);
  }
  const arr = Array.from(seen);
  arr.sort((a, b) =>
    a === "timestamp" ? -1 : b === "timestamp" ? 1 : a.localeCompare(b)
  );
  return arr;
}

function format(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "object") return JSON.stringify(v);
  return String(v);
}
