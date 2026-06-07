/**
 * TablePanelViz — renders a table-type dashboard panel.
 * Reuses packages/react/src/query/results/ResultsGrid (package version with ky- classes).
 */

import { useEffect } from "react";
import { ResultsGrid } from "../../query/results/ResultsGrid";
import { usePanelQuery } from "../usePanelQuery";
import type { TimeRange } from "../../query/time-range/time-range-types";
import type { DashboardPanel } from "@kyma-ai/client";

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
      <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-text-xs ky-text-muted-foreground">
        No query configured.
      </div>
    );
  }

  if (result.kind === "idle" || result.kind === "loading") {
    return (
      <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-text-xs ky-text-muted-foreground ky-animate-pulse">
        Loading…
      </div>
    );
  }

  if (result.kind === "error") {
    return (
      <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-p-2">
        <span className="ky-text-xs ky-text-destructive">{result.message}</span>
      </div>
    );
  }

  if (result.rows.length === 0) {
    return (
      <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-text-xs ky-text-muted-foreground">
        No data.
      </div>
    );
  }

  return (
    <div className="ky-h-full ky-w-full ky-overflow-hidden">
      <ResultsGrid columns={result.columns} rows={result.rows.slice(0, maxRows)} />
    </div>
  );
}
