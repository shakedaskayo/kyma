import { useMemo } from "react";
import { ArrowLeft } from "lucide-react";
import { Button } from "../internal/ui/button";
import { partitionColumns, formatCell } from "./columns";
import type { SourceState } from "./types";

type Props = {
  src: SourceState;
  onBack: () => void;
  onOpenRow: (row: Record<string, unknown>) => void;
};

export function SourceTableView({ src, onBack, onOpenRow }: Props) {
  const { shown: cols, hiddenVectors } = useMemo(() => partitionColumns(src.rows), [src]);
  return (
    <div>
      <div className="pv-flex pv-items-center pv-gap-2 pv-px-3 pv-py-2 pv-border-b">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="pv-size-4 pv-mr-1" /> Back to stream
        </Button>
        <span className="pv-font-mono pv-text-sm">{src.source}</span>
        <span className="pv-text-xs pv-text-muted-foreground pv-tabular-nums">
          {src.total.toLocaleString()} rows · not in timeline (no timestamp column)
        </span>
      </div>
      {src.rows.length === 0 ? (
        <div className="pv-p-3 pv-text-sm pv-text-muted-foreground">
          {src.progress === "done" ? "no rows" : "loading…"}
        </div>
      ) : (
        <table className="pv-w-full pv-text-xs pv-font-mono">
          <thead className="pv-sticky pv-top-0 pv-bg-background pv-z-10">
            <tr>
              {cols.map((c) => (
                <th key={c} className="pv-text-left pv-px-2 pv-py-1 pv-border-b pv-font-medium">{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {src.rows.slice(0, 500).map((r, i) => (
              <tr key={i} className="hover:pv-bg-accent pv-cursor-pointer" onClick={() => onOpenRow(r)}>
                {cols.map((c) => (
                  <td key={c} className="pv-px-2 pv-py-1 pv-truncate pv-max-w-xs pv-align-top" title={formatCell(r[c])}>
                    {formatCell(r[c])}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {hiddenVectors.length > 0 && (
        <div className="pv-p-2 pv-text-xs pv-text-muted-foreground">
          {hiddenVectors.length} vector column{hiddenVectors.length > 1 ? "s" : ""} hidden — open a row to see {hiddenVectors.length > 1 ? "them" : "it"}
        </div>
      )}
    </div>
  );
}
