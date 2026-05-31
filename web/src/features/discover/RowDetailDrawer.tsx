import { X } from "lucide-react";
import { Button } from "@/components/ui/button";
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
        className="fixed inset-0 bg-black/30 z-40"
        onClick={onClose}
        aria-label="close row detail"
      />
      <div
        role="dialog"
        aria-label={`row detail for ${source}`}
        className="fixed top-0 right-0 h-full w-[480px] max-w-full bg-background border-l shadow-lg z-50 overflow-auto"
      >
        <div className="sticky top-0 bg-background border-b flex items-center justify-between px-4 py-3">
          <div className="font-mono text-sm truncate">{source}</div>
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="close">
            <X className="size-4" />
          </Button>
        </div>
        <div className="p-4 space-y-1 font-mono text-xs">
          {Object.entries(row).map(([k, v]) => (
            <div key={k} className="grid grid-cols-[160px_1fr] gap-2 group items-start">
              <div className="text-muted-foreground truncate" title={k}>{k}</div>
              <div className="flex items-start gap-1 min-w-0">
                <span className="break-all flex-1 whitespace-pre-wrap">
                  {format(v)}
                </span>
                {scalarish(v) && (
                  <>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-5 px-1 text-[10px] opacity-0 group-hover:opacity-100"
                      onClick={() => onAddPill({ kind: "eq", field: k, value: String(v) })}
                      aria-label={`filter to ${k} == ${String(v)}`}
                    >
                      =
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      className="h-5 px-1 text-[10px] opacity-0 group-hover:opacity-100"
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
