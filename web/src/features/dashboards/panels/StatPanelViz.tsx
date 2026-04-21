import { useEffect } from "react";
import type { TimeRange } from "@/features/tabs/workspace-store";
import { usePanelQuery } from "@/features/dashboards/useDashboardQuery";
import type { DashboardPanel } from "@/sdk/dashboards";

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

  const firstRow = result.rows[0];
  if (!firstRow) {
    return (
      <div className="flex h-full flex-col items-center justify-center">
        <div className="text-5xl font-bold tabular-nums">—</div>
        {label && <div className="mt-1 text-xs text-muted-foreground">{label}</div>}
      </div>
    );
  }

  // Find the value: prefer valueCol, else first numeric column, else first column
  let rawValue: unknown = undefined;
  if (valueCol && valueCol in firstRow) {
    rawValue = firstRow[valueCol];
  } else {
    // find first numeric column
    const numericCol = result.columns.find((c) => c.kind === "numeric");
    if (numericCol) {
      rawValue = firstRow[numericCol.name];
    } else {
      // fall back to first column
      const firstCol = result.columns[0];
      if (firstCol) rawValue = firstRow[firstCol.name];
    }
  }

  const displayValue = rawValue !== undefined ? formatValue(rawValue, format) : "—";

  return (
    <div className="flex h-full flex-col items-center justify-center gap-1">
      <div className="text-5xl font-bold tabular-nums text-foreground">{displayValue}</div>
      {label && (
        <div className="text-xs text-muted-foreground">{label}</div>
      )}
    </div>
  );
}
