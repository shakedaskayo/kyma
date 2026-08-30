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
      <div className="pv-absolute pv-left-1/2 pv-top-6 pv-z-30 -pv-translate-x-1/2">
        <button
          type="button"
          onClick={() => setOpen(true)}
          aria-label="Search nodes"
          className="pv-glass pv-flex pv-items-center pv-gap-2 pv-rounded-full pv-border pv-border-border pv-px-3 pv-py-1.5 pv-text-xs pv-text-muted-foreground pv-shadow-elev-2 pv-transition-colors hover:pv-text-foreground"
        >
          <Search className="pv-h-3.5 pv-w-3.5" />
          Search nodes…
          <kbd className="pv-rounded pv-border pv-border-border pv-px-1.5 pv-py-0.5 pv-text-2xs">
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
    <div className="pv-absolute pv-left-1/2 pv-top-6 pv-z-30 pv-w-[480px] pv-max-w-[90%] -pv-translate-x-1/2">
      <div className="pv-glass pv-rounded-xl pv-border pv-border-border pv-shadow-elev-3">
        <div className="pv-flex pv-items-center pv-gap-2 pv-border-b pv-border-border pv-px-3 pv-py-2.5">
          <Search className="pv-h-4 pv-w-4 pv-text-muted-foreground" />
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
            className="pv-w-full pv-bg-transparent pv-text-sm pv-text-foreground pv-outline-none placeholder:pv-text-muted-foreground"
          />
          <kbd className="pv-rounded pv-border pv-border-border pv-px-1.5 pv-py-0.5 pv-text-2xs pv-text-muted-foreground">esc</kbd>
        </div>
        {results.length > 0 && (
          <ul className="pv-max-h-80 pv-overflow-y-auto pv-py-1">
            {results.map((r, i) => (
              <li key={r.id}>
                <button
                  type="button"
                  onClick={() => select(r.id)}
                  onMouseEnter={() => setActive(i)}
                  className={`pv-flex pv-w-full pv-items-center pv-gap-2 pv-px-3 pv-py-1.5 pv-text-left pv-text-sm ${
                    i === active ? "pv-bg-accent pv-text-foreground" : "pv-text-muted-foreground"
                  }`}
                >
                  <span
                    className="pv-h-2 pv-w-2 pv-shrink-0 pv-rounded-full"
                    style={{ background: getLabelColor(r.nodeLabel) }}
                  />
                  <span className="pv-truncate">{r.label}</span>
                  <span className="pv-ml-auto pv-shrink-0 pv-text-2xs pv-text-muted-foreground">{r.nodeLabel}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {query.trim() && results.length === 0 && (
          <div className="pv-px-3 pv-py-3 pv-text-xs pv-text-muted-foreground">No matching nodes.</div>
        )}
      </div>
    </div>
  );
}
