import { useMemo, useState } from "react";
import { ChevronRight, ChevronDown, AlertCircle, Clock } from "lucide-react";
import { partitionColumns, formatCell } from "./columns";
import type { SourceState } from "./types";

type Props = {
  src: SourceState;
  /** Whether the page currently has a time range selected. */
  timeRangeActive: boolean;
  onOpenRow: (row: Record<string, unknown>) => void;
};

export function SourceSection({ src, timeRangeActive, onOpenRow }: Props) {
  const [open, setOpen] = useState(true);
  const { shown: cols, hiddenVectors } = useMemo(() => partitionColumns(src.rows), [src]);

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
        {timeRangeActive && !src.hasTimestamp && (
          <span
            className="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] text-muted-foreground"
            title="This source has no timestamp-typed column, so the selected time range does not apply — all matching rows are shown."
          >
            <Clock className="size-3" /> no time filter
          </span>
        )}
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
                        title={formatCell(r[c])}
                      >
                        {formatCell(r[c])}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {(src.rows.length > 200 || hiddenVectors.length > 0) && (
            <div className="p-2 text-xs text-muted-foreground space-x-3">
              {src.rows.length > 200 && (
                <span>showing first 200 of {src.rows.length} loaded rows</span>
              )}
              {hiddenVectors.length > 0 && (
                <span title={hiddenVectors.join(", ")}>
                  {hiddenVectors.length} vector column{hiddenVectors.length > 1 ? "s" : ""} hidden
                  — open a row to see {hiddenVectors.length > 1 ? "them" : "it"}
                </span>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
