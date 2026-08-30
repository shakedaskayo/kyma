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
    <div ref={ref} className="ky-relative">
      <Button variant="outline" size="sm" onClick={() => setOpen(!open)}>
        {label(value, views.data)}
      </Button>
      {open && (
        <div className="ky-absolute ky-top-full ky-left-0 ky-mt-1 ky-w-72 ky-z-50 ky-rounded-md ky-border ky-bg-popover ky-p-2 ky-shadow-md">
          <button
            type="button"
            className="ky-block ky-w-full ky-text-left ky-text-sm hover:ky-bg-accent ky-rounded ky-px-2 ky-py-1.5"
            onClick={() => {
              onChange({ kind: "all" });
              setOpen(false);
            }}
          >
            All sources
          </button>
          <div className="ky-border-t ky-my-2 ky-pt-2">
            <div className="ky-text-xs ky-text-muted-foreground ky-mb-1 ky-px-2">Saved views</div>
            {(views.data ?? []).map((v) => (
              <button
                key={v.id}
                type="button"
                className="ky-block ky-w-full ky-text-left ky-text-sm hover:ky-bg-accent ky-rounded ky-px-2 ky-py-1.5"
                onClick={() => {
                  onChange({ kind: "view", view_id: v.id });
                  setOpen(false);
                }}
              >
                {v.name}
              </button>
            ))}
            {(views.data ?? []).length === 0 && (
              <div className="ky-text-xs ky-text-muted-foreground ky-italic ky-px-2 ky-py-1">
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
