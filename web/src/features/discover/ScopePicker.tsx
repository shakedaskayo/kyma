//! Scope picker — a button that opens a small popover with two options:
//! "All sources" (default) and saved views (loaded via listSavedViews()).
//! The "Pick db/tables" mode is v2 and not built here.

import { useState, useRef, useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { listSavedViews, type SavedView } from "@/sdk/discover";
import type { Scope } from "./types";

type Props = { value: Scope; onChange: (s: Scope) => void };

export function ScopePicker({ value, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const views = useQuery({ queryKey: ["saved-views"], queryFn: listSavedViews });

  // Click-outside to close.
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
    <div ref={ref} className="relative">
      <Button variant="outline" size="sm" onClick={() => setOpen(!open)}>
        {label(value, views.data)}
      </Button>
      {open && (
        <div className="absolute top-full left-0 mt-1 w-72 z-50 rounded-md border bg-popover p-2 shadow-md">
          <button
            type="button"
            className="block w-full text-left text-sm hover:bg-accent rounded px-2 py-1.5"
            onClick={() => {
              onChange({ kind: "all" });
              setOpen(false);
            }}
          >
            All sources
          </button>
          <div className="border-t my-2 pt-2">
            <div className="text-xs text-muted-foreground mb-1 px-2">Saved views</div>
            {(views.data ?? []).map((v) => (
              <button
                key={v.id}
                type="button"
                className="block w-full text-left text-sm hover:bg-accent rounded px-2 py-1.5"
                onClick={() => {
                  onChange({ kind: "view", view_id: v.id });
                  setOpen(false);
                }}
              >
                {v.name}
              </button>
            ))}
            {(views.data ?? []).length === 0 && (
              <div className="text-xs text-muted-foreground italic px-2 py-1">
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
