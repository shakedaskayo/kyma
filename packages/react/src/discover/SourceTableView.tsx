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
      <div className="ky-flex ky-items-center ky-gap-2 ky-px-3 ky-py-2 ky-border-b">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="ky-size-4 ky-mr-1" /> Back to stream
        </Button>
        <span className="ky-font-mono ky-text-sm">{src.source}</span>
        <span className="ky-text-xs ky-text-muted-foreground ky-tabular-nums">
          {src.total.toLocaleString()} rows · not in timeline (no timestamp column)
        </span>
      </div>
      {src.rows.length === 0 ? (
        <div className="ky-p-3 ky-text-sm ky-text-muted-foreground">
          {src.progress === "done" ? "no rows" : "loading…"}
        </div>
      ) : (
        <table className="ky-w-full ky-text-xs ky-font-mono">
          <thead className="ky-sticky ky-top-0 ky-bg-background ky-z-10">
            <tr>
              {cols.map((c) => (
                <th key={c} className="ky-text-left ky-px-2 ky-py-1 ky-border-b ky-font-medium">{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {src.rows.slice(0, 500).map((r, i) => (
              <tr key={i} className="hover:ky-bg-accent ky-cursor-pointer" onClick={() => onOpenRow(r)}>
                {cols.map((c) => (
                  <td key={c} className="ky-px-2 ky-py-1 ky-truncate ky-max-w-xs ky-align-top" title={formatCell(r[c])}>
                    {formatCell(r[c])}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {hiddenVectors.length > 0 && (
        <div className="ky-p-2 ky-text-xs ky-text-muted-foreground">
          {hiddenVectors.length} vector column{hiddenVectors.length > 1 ? "s" : ""} hidden — open a row to see {hiddenVectors.length > 1 ? "them" : "it"}
        </div>
      )}
    </div>
  );
}
