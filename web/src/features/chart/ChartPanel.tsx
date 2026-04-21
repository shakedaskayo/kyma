import ReactECharts from "echarts-for-react";
import { useMemo } from "react";
import { autoChartAxes, type Column } from "@/sdk/arrow";

export function ChartPanel({ columns, rows }: { columns: Column[]; rows: Record<string, unknown>[] }) {
  const option = useMemo(() => {
    const spec = autoChartAxes(columns);
    if (spec.type === "none")
      return null;
    if (spec.type === "stat") {
      return null; // stat rendered as text below (outside ECharts)
    }
    const x = rows.map((r) => r[spec.x!]);
    const y = rows.map((r) => Number(r[spec.y]));

    if (spec.type === "line" || spec.type === "scatter") {
      return {
        animation: false,
        tooltip: { trigger: "axis" },
        xAxis: { type: spec.type === "line" ? "time" : "value", name: spec.x },
        yAxis: { type: "value", name: spec.y },
        series: [{ name: spec.y, type: spec.type === "line" ? "line" : "scatter", data: rows.map((r) => [r[spec.x!], r[spec.y]]), showSymbol: false }],
      };
    }
    // bar
    return {
      animation: false,
      tooltip: { trigger: "axis" },
      xAxis: { type: "category", data: x, name: spec.x },
      yAxis: { type: "value", name: spec.y },
      series: [{ name: spec.y, type: "bar", data: y }],
    };
  }, [columns, rows]);

  if (!option) {
    const spec = autoChartAxes(columns);
    if (spec.type === "stat") {
      const v = rows.at(-1)?.[spec.y];
      return (
        <div className="flex h-full flex-col items-center justify-center">
          <div className="text-sm text-muted-foreground">{spec.y}</div>
          <div className="text-5xl font-bold tabular-nums">{String(v ?? "—")}</div>
        </div>
      );
    }
    return <div className="p-6 text-sm text-muted-foreground">No auto-chart for this shape.</div>;
  }
  return <ReactECharts option={option} style={{ height: "100%", width: "100%" }} theme="default" notMerge={true} />;
}
