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
      className={`ky-flex ky-items-center ky-gap-2 ky-px-2 ky-py-1 ky-rounded ky-text-sm ky-cursor-pointer hover:ky-bg-accent ${isSelected ? "ky-bg-accent" : ""}`}
      onClick={onClick}
    >
      {showCheckbox ? (
        <input
          type="checkbox"
          checked={isVisible}
          onChange={(e) => { e.stopPropagation(); onToggleVisible(); }}
          onClick={(e) => e.stopPropagation()}
          className="ky-size-3.5 ky-cursor-pointer"
          aria-label={`toggle visibility of ${s.source}`}
        />
      ) : (
        <Table2 className="ky-size-3.5 ky-text-muted-foreground" />
      )}
      <span className="ky-truncate ky-flex-1 ky-font-mono ky-text-xs" title={s.source}>
        {s.source}
      </span>
      {s.progress === "running" && <Loader2 className="ky-size-3 ky-animate-spin ky-text-muted-foreground" />}
      {s.progress === "done" && <CheckCircle2 className="ky-size-3 ky-text-muted-foreground" />}
      {s.progress === "error" && <AlertCircle className="ky-size-3 ky-text-destructive" />}
      <span className="ky-text-xs ky-text-muted-foreground ky-tabular-nums">{s.total.toLocaleString()}</span>
    </div>
  );
}

export function SourcesRail({
  results, visible, onToggleVisible, selected, onSelect, onOpenTable,
}: Props) {
  const all = Array.from(results.sources.values());
  if (all.length === 0 && results.status === "idle") {
    return <div className="ky-text-xs ky-text-muted-foreground ky-p-2">Run a search.</div>;
  }
  const inTimeline = all.filter((s) => s.timestampColumn != null);
  const noTimeline = all.filter((s) => s.timestampColumn == null);

  return (
    <div className="ky-space-y-0.5">
      <div className="ky-text-xs ky-font-medium ky-text-muted-foreground ky-px-2 ky-py-1 ky-uppercase ky-tracking-wide">
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
            className="ky-text-xs ky-font-medium ky-text-muted-foreground ky-px-2 ky-pt-2 ky-pb-1 ky-uppercase ky-tracking-wide"
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
