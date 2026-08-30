/**
 * CommandBar — glass search panel, top-center. Opened from the always-visible
 * search pill or ⌘K/Ctrl+K. One box for: node search with fly-to
 * (Enter/click), arrow-key navigation, and Esc to close. Searches the
 * already-loaded graphology instance (no server round-trip — the full graph
 * is local).
 */
import { useEffect, useMemo, useRef, useState } from "react";
import type Graph from "graphology";
import { Search } from "lucide-react";
import { getLabelColor } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";

const MAX_RESULTS = 20;

export function CommandBar({ graphRef }: { graphRef: React.RefObject<Graph | null> }) {
  const open = useGraphStore((s) => s.commandBarOpen);
  const setOpen = useGraphStore((s) => s.setCommandBarOpen);
  const focusNode = useGraphStore((s) => s.focusNode);
  const pushTrail = useGraphStore((s) => s.pushTrail);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Global ⌘K / Ctrl+K.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen(!open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, setOpen]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const results = useMemo(() => {
    const graph = graphRef.current;
    const q = query.trim().toLowerCase();
    if (!graph || !q) return [];
    const out: Array<{ id: string; label: string; nodeLabel: string }> = [];
    graph.forEachNode((id, attrs) => {
      if (out.length >= MAX_RESULTS) return;
      const label = String(attrs.label ?? "");
      if (label.toLowerCase().includes(q) || id.toLowerCase().includes(q)) {
        out.push({ id, label, nodeLabel: String(attrs.nodeLabel ?? "Node") });
      }
    });
    return out;
  }, [graphRef, query]);

  if (!open) {
    const isMac = typeof navigator !== "undefined" && /mac/i.test(navigator.platform);
    return (
      <div className="ky-absolute ky-left-1/2 ky-top-6 ky-z-30 -ky-translate-x-1/2">
        <button
          type="button"
          onClick={() => setOpen(true)}
          aria-label="Search nodes"
          className="ky-glass ky-flex ky-items-center ky-gap-2 ky-rounded-full ky-border ky-border-border ky-px-3 ky-py-1.5 ky-text-xs ky-text-muted-foreground ky-shadow-elev-2 ky-transition-colors hover:ky-text-foreground"
        >
          <Search className="ky-h-3.5 ky-w-3.5" />
          Search nodes…
          <kbd className="ky-rounded ky-border ky-border-border ky-px-1.5 ky-py-0.5 ky-text-2xs">
            {isMac ? "⌘K" : "Ctrl K"}
          </kbd>
        </button>
      </div>
    );
  }

  const select = (id: string) => {
    pushTrail(id);
    focusNode(id); // fly-to via focusSeq
    setOpen(false);
  };

  return (
    <div className="ky-absolute ky-left-1/2 ky-top-6 ky-z-30 ky-w-[480px] ky-max-w-[90%] -ky-translate-x-1/2">
      <div className="ky-glass ky-rounded-xl ky-border ky-border-border ky-shadow-elev-3">
        <div className="ky-flex ky-items-center ky-gap-2 ky-border-b ky-border-border ky-px-3 ky-py-2.5">
          <Search className="ky-h-4 ky-w-4 ky-text-muted-foreground" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") setOpen(false);
              else if (e.key === "ArrowDown") setActive((a) => Math.min(a + 1, results.length - 1));
              else if (e.key === "ArrowUp") setActive((a) => Math.max(a - 1, 0));
              else if (e.key === "Enter" && results[active]) select(results[active].id);
            }}
            placeholder="Search nodes, fly to anything…"
            className="ky-w-full ky-bg-transparent ky-text-sm ky-text-foreground ky-outline-none placeholder:ky-text-muted-foreground"
          />
          <kbd className="ky-rounded ky-border ky-border-border ky-px-1.5 ky-py-0.5 ky-text-2xs ky-text-muted-foreground">esc</kbd>
        </div>
        {results.length > 0 && (
          <ul className="ky-max-h-80 ky-overflow-y-auto ky-py-1">
            {results.map((r, i) => (
              <li key={r.id}>
                <button
                  type="button"
                  onClick={() => select(r.id)}
                  onMouseEnter={() => setActive(i)}
                  className={`ky-flex ky-w-full ky-items-center ky-gap-2 ky-px-3 ky-py-1.5 ky-text-left ky-text-sm ${
                    i === active ? "ky-bg-accent ky-text-foreground" : "ky-text-muted-foreground"
                  }`}
                >
                  <span
                    className="ky-h-2 ky-w-2 ky-shrink-0 ky-rounded-full"
                    style={{ background: getLabelColor(r.nodeLabel) }}
                  />
                  <span className="ky-truncate">{r.label}</span>
                  <span className="ky-ml-auto ky-shrink-0 ky-text-2xs ky-text-muted-foreground">{r.nodeLabel}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {query.trim() && results.length === 0 && (
          <div className="ky-px-3 ky-py-3 ky-text-xs ky-text-muted-foreground">No matching nodes.</div>
        )}
      </div>
    </div>
  );
}
