import { useEffect } from "react";
import { ResultsGrid } from "@/features/results-grid/ResultsGrid";
import type { TimeRange } from "@/features/tabs/workspace-store";
import { usePanelQuery } from "@/features/dashboards/useDashboardQuery";
import type { DashboardPanel } from "@/sdk/dashboards";

interface Props {
  panel: DashboardPanel;
  timeRange: TimeRange;
}

export function TablePanelViz({ panel, timeRange }: Props) {
  const { result, run } = usePanelQuery();
  const maxRows = (panel.config.maxRows as number | undefined) ?? 50;

  useEffect(() => {
    if (panel.query) {
      void run(panel.query, panel.database_name, timeRange);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [panel.query, panel.database_name, timeRange.preset, timeRange.from, timeRange.to]);

  if (!panel.query) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
        No query configured.
      </div>
    );
  }

  if (result.kind === "idle" || result.kind === "loading") {
    return (
      <div className="flex h-full items-center justify-center text-xs text-muted-foreground animate-pulse">
        Loading…
      </div>
    );
  }

  if (result.kind === "error") {
    return (
      <div className="flex h-full items-center justify-center p-2">
        <span className="text-xs text-destructive">{result.message}</span>
      </div>
    );
  }

  if (result.rows.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
        No data.
      </div>
    );
  }

  return (
    <div className="h-full w-full overflow-hidden">
      <ResultsGrid columns={result.columns} rows={result.rows.slice(0, maxRows)} />
    </div>
  );
}
