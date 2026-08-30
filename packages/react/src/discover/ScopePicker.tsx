// ScopePicker — a button that opens a small popover with scope options:
// "All sources" (default) and saved views (loaded via client.discover.listSavedViews).

import { useState, useRef, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../internal/ui/button";
import { usePensieveClient } from "../provider/context";
import type { SavedView, Scope } from "@pensieve-ai/client";

type Props = { value: Scope; onChange: (s: Scope) => void };

export function ScopePicker({ value, onChange }: Props) {
  const client = usePensieveClient();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const views = useQuery({
    queryKey: ["discover-saved-views"],
    queryFn: () => client.discover.listSavedViews(),
  });

  useEffect(() => {
    if (!open) return;
    function onDocClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [open]);

  return (
    <div ref={ref} className="pv-relative">
      <Button variant="outline" size="sm" onClick={() => setOpen(!open)}>
        {label(value, views.data)}
      </Button>
      {open && (
        <div className="pv-absolute pv-top-full pv-left-0 pv-mt-1 pv-w-72 pv-z-50 pv-rounded-md pv-border pv-bg-popover pv-p-2 pv-shadow-md">
          <button
            type="button"
            className="pv-block pv-w-full pv-text-left pv-text-sm hover:pv-bg-accent pv-rounded pv-px-2 pv-py-1.5"
            onClick={() => {
              onChange({ kind: "all" });
              setOpen(false);
            }}
          >
            All sources
          </button>
          <div className="pv-border-t pv-my-2 pv-pt-2">
            <div className="pv-text-xs pv-text-muted-foreground pv-mb-1 pv-px-2">Saved views</div>
            {(views.data ?? []).map((v) => (
              <button
                key={v.id}
                type="button"
                className="pv-block pv-w-full pv-text-left pv-text-sm hover:pv-bg-accent pv-rounded pv-px-2 pv-py-1.5"
                onClick={() => {
                  onChange({ kind: "view", view_id: v.id });
                  setOpen(false);
                }}
              >
                {v.name}
              </button>
            ))}
            {(views.data ?? []).length === 0 && (
              <div className="pv-text-xs pv-text-muted-foreground pv-italic pv-px-2 pv-py-1">
                No saved views yet.
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function label(s: Scope, views?: SavedView[]): string {
  if (s.kind === "all") return "All sources";
  if (s.kind === "sources") return `${s.sources.length} sources`;
  const v = views?.find((vv) => vv.id === s.view_id);
  return v ? `View: ${v.name}` : "Saved view";
}
