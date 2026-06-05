import { Loader2, AlertCircle, CheckCircle2, Table2 } from "lucide-react";
import type { DiscoverResultsState, SourceKey, SourceState } from "./types";

type Props = {
  results: DiscoverResultsState;
  visible: SourceKey[] | null; // null = all visible
  onToggleVisible: (s: SourceKey) => void;
  selected: SourceKey | null;
  onSelect: (s: SourceKey) => void;
  onOpenTable: (s: SourceKey) => void;
};

function Row({
  s, isVisible, isSelected, onToggleVisible, onClick, showCheckbox,
}: {
  s: SourceState;
  isVisible: boolean;
  isSelected: boolean;
  onToggleVisible: () => void;
  onClick: () => void;
  showCheckbox: boolean;
}) {
  return (
    <div
      className={`flex items-center gap-2 px-2 py-1 rounded text-sm cursor-pointer hover:bg-accent ${isSelected ? "bg-accent" : ""}`}
      onClick={onClick}
    >
      {showCheckbox ? (
        <input
          type="checkbox"
          checked={isVisible}
          onChange={(e) => { e.stopPropagation(); onToggleVisible(); }}
          onClick={(e) => e.stopPropagation()}
          className="size-3.5 accent-foreground cursor-pointer"
          aria-label={`toggle visibility of ${s.source}`}
        />
      ) : (
        <Table2 className="size-3.5 text-muted-foreground" />
      )}
      <span className="truncate flex-1 font-mono text-xs" title={s.source}>
        {s.source}
      </span>
      {s.progress === "running" && <Loader2 className="size-3 animate-spin text-muted-foreground" />}
      {s.progress === "done" && <CheckCircle2 className="size-3 text-muted-foreground" />}
      {s.progress === "error" && <AlertCircle className="size-3 text-destructive" />}
      <span className="text-xs text-muted-foreground tabular-nums">{s.total.toLocaleString()}</span>
    </div>
  );
}

export function SourcesRail({
  results, visible, onToggleVisible, selected, onSelect, onOpenTable,
}: Props) {
  const all = Array.from(results.sources.values());
  if (all.length === 0 && results.status === "idle") {
    return <div className="text-xs text-muted-foreground p-2">Run a search.</div>;
  }
  const inTimeline = all.filter((s) => s.timestampColumn != null);
  const noTimeline = all.filter((s) => s.timestampColumn == null);

  return (
    <div className="space-y-0.5">
      <div className="text-xs font-medium text-muted-foreground px-2 py-1 uppercase tracking-wide">
        Sources
      </div>
      {inTimeline.map((s) => (
        <Row
          key={s.source}
          s={s}
          showCheckbox
          isVisible={visible == null || visible.includes(s.source)}
          isSelected={selected === s.source}
          onToggleVisible={() => onToggleVisible(s.source)}
          onClick={() => onSelect(s.source)}
        />
      ))}
      {noTimeline.length > 0 && (
        <>
          <div
            className="text-xs font-medium text-muted-foreground px-2 pt-2 pb-1 uppercase tracking-wide"
            title="These sources have no timestamp-typed column, so they can't join the timeline. Click one to view it as a table."
          >
            Not in timeline
          </div>
          {noTimeline.map((s) => (
            <Row
              key={s.source}
              s={s}
              showCheckbox={false}
              isVisible
              isSelected={selected === s.source}
              onToggleVisible={() => {}}
              onClick={() => { onSelect(s.source); onOpenTable(s.source); }}
            />
          ))}
        </>
      )}
    </div>
  );
}
