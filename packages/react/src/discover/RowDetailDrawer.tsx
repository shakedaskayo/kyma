import { X } from "lucide-react";
import { Button } from "../internal/ui/button";
import type { Pill, SourceKey } from "./types";

type Props = {
  source: SourceKey | null;
  row: Record<string, unknown> | null;
  onClose: () => void;
  onAddPill: (p: Pill) => void;
};

export function RowDetailDrawer({ source, row, onClose, onAddPill }: Props) {
  if (!row) return null;
  return (
    <>
      <div
        className="pv-fixed pv-inset-0 pv-bg-black/30 pv-z-40"
        onClick={onClose}
        aria-label="close row detail"
      />
      <div
        role="dialog"
        aria-label={`row detail for ${source}`}
        className="pv-fixed pv-top-0 pv-right-0 pv-h-full pv-w-[480px] pv-max-w-full pv-bg-background pv-border-l pv-shadow-lg pv-z-50 pv-overflow-auto"
      >
        <div className="pv-sticky pv-top-0 pv-bg-background pv-border-b pv-flex pv-items-center pv-justify-between pv-px-4 pv-py-3">
          <div className="pv-font-mono pv-text-sm pv-truncate">{source}</div>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="close">
            <X className="pv-size-4" />
          </Button>
        </div>
        <div className="pv-p-4 pv-space-y-1 pv-font-mono pv-text-xs">
          {Object.entries(row).map(([k, v]) => (
            <div key={k} className="pv-grid pv-grid-cols-[160px_1fr] pv-gap-2 pv-group pv-items-start">
              <div className="pv-text-muted-foreground pv-truncate" title={k}>{k}</div>
              <div className="pv-flex pv-items-start pv-gap-1 pv-min-w-0">
                <span className="pv-break-all pv-flex-1 pv-whitespace-pre-wrap">
                  {format(v)}
                </span>
                {scalarish(v) && (
                  <>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="pv-h-5 pv-px-1 pv-text-[10px] pv-opacity-0 group-hover:pv-opacity-100"
                      onClick={() => onAddPill({ kind: "eq", field: k, value: String(v) })}
                      aria-label={`filter to ${k} == ${String(v)}`}
                    >
                      =
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="pv-h-5 pv-px-1 pv-text-[10px] pv-opacity-0 group-hover:pv-opacity-100"
                      onClick={() => onAddPill({ kind: "neq", field: k, value: String(v) })}
                      aria-label={`filter to ${k} != ${String(v)}`}
                    >
                      ≠
                    </Button>
                  </>
                )}
              </div>
            </div>
          ))}
        </div>
      </div>
    </>
  );
}

function format(v: unknown): string {
  if (v == null) return "null";
  if (typeof v === "object") return JSON.stringify(v, null, 2);
  return String(v);
}

function scalarish(v: unknown): boolean {
  return v != null && (typeof v === "string" || typeof v === "number" || typeof v === "boolean");
}
