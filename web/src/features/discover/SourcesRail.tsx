import { Loader2, AlertCircle, CheckCircle2 } from "lucide-react";
import type { DiscoverResultsState, SourceKey } from "./types";

type Props = {
  results: DiscoverResultsState;
  visible: SourceKey[] | null;          // null = all visible
  onToggleVisible: (s: SourceKey) => void;
  selected: SourceKey | null;
  onSelect: (s: SourceKey) => void;
};

export function SourcesRail({
  results, visible, onToggleVisible, selected, onSelect,
}: Props) {
  const sources = Array.from(results.sources.values());
  if (sources.length === 0 && results.status === "idle") {
    return <div className="text-xs text-muted-foreground p-2">Run a search.</div>;
  }
  return (
    <div className="space-y-0.5">
      <div className="text-xs font-medium text-muted-foreground px-2 py-1 uppercase tracking-wide">
        Sources
      </div>
      {sources.map((s) => {
        const isVisible = visible == null || visible.includes(s.source);
        const isSelected = selected === s.source;
        return (
          <div
            key={s.source}
            className={`flex items-center gap-2 px-2 py-1 rounded text-sm cursor-pointer hover:bg-accent ${isSelected ? "bg-accent" : ""}`}
            onClick={() => onSelect(s.source)}
          >
            <input
              type="checkbox"
              checked={isVisible}
              onChange={(e) => { e.stopPropagation(); onToggleVisible(s.source); }}
              onClick={(e) => e.stopPropagation()}
              className="size-3.5 accent-foreground cursor-pointer"
              aria-label={`toggle visibility of ${s.source}`}
            />
            <span className="truncate flex-1 font-mono text-xs" title={s.source}>
              {s.source}
            </span>
            {s.progress === "running" && (
              <Loader2 className="size-3 animate-spin text-muted-foreground" />
            )}
            {s.progress === "done" && (
              <CheckCircle2 className="size-3 text-muted-foreground" />
            )}
            {s.progress === "error" && (
              <AlertCircle className="size-3 text-destructive" />
            )}
            <span className="text-xs text-muted-foreground tabular-nums">
              {s.total.toLocaleString()}
            </span>
          </div>
        );
      })}
    </div>
  );
}
