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
        className="ky-fixed ky-inset-0 ky-bg-black/30 ky-z-40"
        onClick={onClose}
        aria-label="close row detail"
      />
      <div
        role="dialog"
        aria-label={`row detail for ${source}`}
        className="ky-fixed ky-top-0 ky-right-0 ky-h-full ky-w-[480px] ky-max-w-full ky-bg-background ky-border-l ky-shadow-lg ky-z-50 ky-overflow-auto"
      >
        <div className="ky-sticky ky-top-0 ky-bg-background ky-border-b ky-flex ky-items-center ky-justify-between ky-px-4 ky-py-3">
          <div className="ky-font-mono ky-text-sm ky-truncate">{source}</div>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="close">
            <X className="ky-size-4" />
          </Button>
        </div>
        <div className="ky-p-4 ky-space-y-1 ky-font-mono ky-text-xs">
          {Object.entries(row).map(([k, v]) => (
            <div key={k} className="ky-grid ky-grid-cols-[160px_1fr] ky-gap-2 ky-group ky-items-start">
              <div className="ky-text-muted-foreground ky-truncate" title={k}>{k}</div>
              <div className="ky-flex ky-items-start ky-gap-1 ky-min-w-0">
                <span className="ky-break-all ky-flex-1 ky-whitespace-pre-wrap">
                  {format(v)}
                </span>
                {scalarish(v) && (
                  <>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="ky-h-5 ky-px-1 ky-text-[10px] ky-opacity-0 group-hover:ky-opacity-100"
                      onClick={() => onAddPill({ kind: "eq", field: k, value: String(v) })}
                      aria-label={`filter to ${k} == ${String(v)}`}
                    >
                      =
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="ky-h-5 ky-px-1 ky-text-[10px] ky-opacity-0 group-hover:ky-opacity-100"
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
