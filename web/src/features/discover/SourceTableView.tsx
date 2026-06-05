import { useMemo } from "react";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
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
      <div className="flex items-center gap-2 px-3 py-2 border-b">
        <Button variant="ghost" size="sm" onClick={onBack}>
          <ArrowLeft className="size-4 mr-1" /> Back to stream
        </Button>
        <span className="font-mono text-sm">{src.source}</span>
        <span className="text-xs text-muted-foreground tabular-nums">
          {src.total.toLocaleString()} rows · not in timeline (no timestamp column)
        </span>
      </div>
      {src.rows.length === 0 ? (
        <div className="p-3 text-sm text-muted-foreground">
          {src.progress === "done" ? "no rows" : "loading…"}
        </div>
      ) : (
        <table className="w-full text-xs font-mono">
          <thead className="sticky top-0 bg-background z-10">
            <tr>
              {cols.map((c) => (
                <th key={c} className="text-left px-2 py-1 border-b font-medium">{c}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {src.rows.slice(0, 500).map((r, i) => (
              <tr key={i} className="hover:bg-accent cursor-pointer" onClick={() => onOpenRow(r)}>
                {cols.map((c) => (
                  <td key={c} className="px-2 py-1 truncate max-w-xs align-top" title={formatCell(r[c])}>
                    {formatCell(r[c])}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {hiddenVectors.length > 0 && (
        <div className="p-2 text-xs text-muted-foreground">
          {hiddenVectors.length} vector column{hiddenVectors.length > 1 ? "s" : ""} hidden — open a row to see {hiddenVectors.length > 1 ? "them" : "it"}
        </div>
      )}
    </div>
  );
}
