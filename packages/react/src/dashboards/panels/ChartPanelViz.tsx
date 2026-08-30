/**
 * ChartPanelViz — renders a chart-type dashboard panel.
 * Reuses packages/react/src/query/chart/ChartPanel (package version with pv- classes).
 */

import { useEffect } from "react";
import { ChartPanel } from "../../query/chart/ChartPanel";
import { usePanelQuery } from "../usePanelQuery";
import type { TimeRange } from "../../query/time-range/time-range-types";
import type { DashboardPanel } from "@pensieve-ai/client";

interface Props {
  panel: DashboardPanel;
  timeRange: TimeRange;
}

export function ChartPanelViz({ panel, timeRange }: Props) {
  const { result, run } = usePanelQuery();

  useEffect(() => {
    if (panel.query) {
      void run(panel.query, panel.database_name, timeRange);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [panel.query, panel.database_name, timeRange.preset, timeRange.from, timeRange.to]);

  if (!panel.query) {
    return (
      <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-text-xs pv-text-muted-foreground">
        No query configured.
      </div>
    );
  }

  if (result.kind === "idle" || result.kind === "loading") {
    return (
      <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-text-xs pv-text-muted-foreground pv-animate-pulse">
        Loading…
      </div>
    );
  }

  if (result.kind === "error") {
    return (
      <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-p-2">
        <span className="pv-text-xs pv-text-destructive">{result.message}</span>
      </div>
    );
  }

  if (result.rows.length === 0) {
    return (
      <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-text-xs pv-text-muted-foreground">
        No data.
      </div>
    );
  }

  return (
    <div className="pv-h-full pv-w-full">
      <ChartPanel columns={result.columns} rows={result.rows} />
    </div>
  );
}
