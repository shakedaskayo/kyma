import { Loader2, AlertCircle, CheckCircle2, Table2 } from "lucide-react";
import type { DiscoverResultsState, SourceKey, SourceState } from "./types";

type Props = {
  results: DiscoverResultsState;
  visible: SourceKey[] | null;
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
      className={`pv-flex pv-items-center pv-gap-2 pv-px-2 pv-py-1 pv-rounded pv-text-sm pv-cursor-pointer hover:pv-bg-accent ${isSelected ? "pv-bg-accent" : ""}`}
      onClick={onClick}
    >
      {showCheckbox ? (
        <input
          type="checkbox"
          checked={isVisible}
          onChange={(e) => { e.stopPropagation(); onToggleVisible(); }}
          onClick={(e) => e.stopPropagation()}
          className="pv-size-3.5 pv-cursor-pointer"
          aria-label={`toggle visibility of ${s.source}`}
        />
      ) : (
        <Table2 className="pv-size-3.5 pv-text-muted-foreground" />
      )}
      <span className="pv-truncate pv-flex-1 pv-font-mono pv-text-xs" title={s.source}>
        {s.source}
      </span>
      {s.progress === "running" && <Loader2 className="pv-size-3 pv-animate-spin pv-text-muted-foreground" />}
      {s.progress === "done" && <CheckCircle2 className="pv-size-3 pv-text-muted-foreground" />}
      {s.progress === "error" && <AlertCircle className="pv-size-3 pv-text-destructive" />}
      <span className="pv-text-xs pv-text-muted-foreground pv-tabular-nums">{s.total.toLocaleString()}</span>
    </div>
  );
}

export function SourcesRail({
  results, visible, onToggleVisible, selected, onSelect, onOpenTable,
}: Props) {
  const all = Array.from(results.sources.values());
  if (all.length === 0 && results.status === "idle") {
    return <div className="pv-text-xs pv-text-muted-foreground pv-p-2">Run a search.</div>;
  }
  const inTimeline = all.filter((s) => s.timestampColumn != null);
  const noTimeline = all.filter((s) => s.timestampColumn == null);

  return (
    <div className="pv-space-y-0.5">
      <div className="pv-text-xs pv-font-medium pv-text-muted-foreground pv-px-2 pv-py-1 pv-uppercase pv-tracking-wide">
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
            className="pv-text-xs pv-font-medium pv-text-muted-foreground pv-px-2 pv-pt-2 pv-pb-1 pv-uppercase pv-tracking-wide"
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
