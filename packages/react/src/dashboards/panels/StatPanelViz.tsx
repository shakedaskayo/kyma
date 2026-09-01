/**
 * StatPanelViz — renders a stat (single-value KPI) dashboard panel.
 * Adapted from web's StatPanelViz with pv- CSS class prefix.
 */

import { useEffect } from "react";
import { usePanelQuery } from "../usePanelQuery";
import type { TimeRange } from "../../query/time-range/time-range-types";
import type { DashboardPanel } from "@pensieve-ai/client";

interface Props {
  panel: DashboardPanel;
  timeRange: TimeRange;
}

type StatFormat = "number" | "duration_ms" | "percent" | "bytes";

function formatValue(value: unknown, format: StatFormat): string {
  const n = typeof value === "number" ? value : parseFloat(String(value));
  if (isNaN(n)) return String(value ?? "—");

  switch (format) {
    case "number":
      return n.toLocaleString(undefined, { maximumFractionDigits: 2 });
    case "duration_ms": {
      if (n < 1000) return `${n.toFixed(0)} ms`;
      if (n < 60_000) return `${(n / 1000).toFixed(1)} s`;
      return `${(n / 60_000).toFixed(1)} min`;
    }
    case "percent":
      return `${n.toFixed(1)}%`;
    case "bytes": {
      if (n < 1024) return `${n.toFixed(0)} B`;
      if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
      if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
      return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
    }
    default:
      return n.toLocaleString();
  }
}

export function StatPanelViz({ panel, timeRange }: Props) {
  const { result, run } = usePanelQuery();
  const label = panel.config.label as string | undefined;
  const valueCol = panel.config.valueCol as string | undefined;
  const format: StatFormat = (panel.config.format as StatFormat | undefined) ?? "number";

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

  const firstRow = result.rows[0];
  if (!firstRow) {
    return (
      <div className="pv-flex pv-h-full pv-flex-col pv-items-center pv-justify-center">
        <div className="pv-text-5xl pv-font-bold pv-tabular-nums">—</div>
        {label && <div className="pv-mt-1 pv-text-xs pv-text-muted-foreground">{label}</div>}
      </div>
    );
  }

  // Prefer valueCol, else first numeric column, else first column
  let rawValue: unknown = undefined;
  if (valueCol && valueCol in firstRow) {
    rawValue = firstRow[valueCol];
  } else {
    const numericCol = result.columns.find((c) => c.kind === "numeric");
    if (numericCol) {
      rawValue = firstRow[numericCol.name];
    } else {
      const firstCol = result.columns[0];
      if (firstCol) rawValue = firstRow[firstCol.name];
    }
  }

  const displayValue = rawValue !== undefined ? formatValue(rawValue, format) : "—";

  return (
    <div className="pv-flex pv-h-full pv-flex-col pv-items-center pv-justify-center pv-gap-1">
      <div className="pv-text-5xl pv-font-bold pv-tabular-nums pv-text-foreground">{displayValue}</div>
      {label && (
        <div className="pv-text-xs pv-text-muted-foreground">{label}</div>
      )}
    </div>
  );
}
