import ReactECharts from "echarts-for-react";
import { useMemo } from "react";
import { BarChart3 } from "lucide-react";
import { autoChartAxes, type Column } from "@pensieve-ai/client";
import { usePensieveContext } from "../../provider/context";
import { DATA_PALETTE, chartTheme } from "../../internal/data-palette";

// ── time formatter ────────────────────────────────────────────────────────────
const MONTHS = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];

function pad2(n: number) { return String(n).padStart(2, "0"); }

/** Pick a display format based on the span of the data (ms). */
function makeAxisFmt(spanMs: number) {
  return (v: number) => {
    const d = new Date(v);
    const hh = pad2(d.getHours()), mm = pad2(d.getMinutes()), ss = pad2(d.getSeconds());
    if (spanMs <= 60_000)            return `${hh}:${mm}:${ss}`;   // <1 min  → HH:mm:ss
    if (spanMs <= 24 * 3600_000)     return `${hh}:${mm}`;          // <1 day  → HH:mm
    const mon = MONTHS[d.getMonth()], day = pad2(d.getDate());
    return `${mon} ${day} ${hh}:${mm}`;                             // ≥1 day  → MMM dd HH:mm
  };
}

/** Tooltip formatter for time-axis charts */
function makeTooltipFmt(yName: string) {
  return (params: { axisValue?: number | string; seriesName?: string; value?: unknown[] | number | string }[]) => {
    if (!params || params.length === 0) return "";
    const p = params[0];
    const ts = typeof p.axisValue === "number"
      ? (() => {
          const d = new Date(p.axisValue);
          const mon = MONTHS[d.getMonth()], day = pad2(d.getDate());
          const hh = pad2(d.getHours()), mm = pad2(d.getMinutes()), ss = pad2(d.getSeconds());
          return `${mon} ${day} ${hh}:${mm}:${ss}`;
        })()
      : String(p.axisValue ?? "");

    const lines = params.map((pp) => {
      const val = Array.isArray(pp.value) ? pp.value[1] : pp.value;
      return `<span style="font-weight:600">${pp.seriesName ?? yName}</span>: ${val}`;
    });
    return `<div style="font-size:11px;line-height:1.6">${ts}<br/>${lines.join("<br/>")}</div>`;
  };
}

// ── component ─────────────────────────────────────────────────────────────────

export function ChartPanel({ columns, rows }: { columns: Column[]; rows: Record<string, unknown>[] }) {
  // Theme-aware: derive isDark from PensieveProvider context (no web-app Zustand store).
  const { isDark } = usePensieveContext();

  const option = useMemo(() => {
    const spec = autoChartAxes(columns);
    if (spec.type === "none" || spec.type === "stat") return null;

    const isSparse = rows.length < 50;
    const symbol = isSparse ? "circle" : "none";
    const symbolSize = 4;

    // Theme-aware axis + tooltip + gridline colors sourced from the Pensieve CSS
    // custom properties (--pensieve-*) via chartTheme(). Falls back gracefully when
    // no DOM is available (tests/SSR).
    const theme = chartTheme(isDark);
    const axisLabelColor = theme.axis;
    const axisLineColor = theme.grid;
    const splitLine = { lineStyle: { color: theme.grid } };
    const tooltipStyle = {
      backgroundColor: theme.tooltipBg,
      borderColor: theme.tooltipBorder,
      textStyle: { color: theme.tooltipText },
    };
    const axis = {
      axisLabel: { color: axisLabelColor },
      axisLine: { lineStyle: { color: axisLineColor } },
      nameTextStyle: { color: axisLabelColor },
    };

    if (spec.type === "line" || spec.type === "scatter") {
      const data = rows.map((r) => [r[spec.x!], r[spec.y]]);
      const times = rows.map((r) => Number(r[spec.x!])).filter(Boolean);
      const spanMs = times.length >= 2 ? Math.max(...times) - Math.min(...times) : 86400_000;
      const axisFmt = makeAxisFmt(spanMs);

      return {
        color: DATA_PALETTE,
        animation: false,
        tooltip: {
          trigger: "axis",
          formatter: makeTooltipFmt(spec.y),
          ...tooltipStyle,
        },
        legend: { show: false, textStyle: { color: axisLabelColor } },
        grid: { left: 60, right: 20, top: 16, bottom: 40, containLabel: false },
        xAxis: {
          type: spec.type === "line" ? "time" : "value",
          name: spec.x,
          axisLabel: { ...(spec.type === "line" ? { formatter: axisFmt } : {}), color: axisLabelColor },
          axisLine: { lineStyle: { color: axisLineColor } },
          nameTextStyle: { color: axisLabelColor },
        },
        yAxis: { type: "value", name: spec.y, splitLine, ...axis },
        series: [{
          name: spec.y,
          type: spec.type === "line" ? "line" : "scatter",
          data,
          showSymbol: isSparse,
          symbol,
          symbolSize,
          smooth: false,
        }],
      };
    }

    // bar
    const x = rows.map((r) => r[spec.x!]);
    const y = rows.map((r) => Number(r[spec.y]));
    return {
      color: DATA_PALETTE,
      animation: false,
      tooltip: { trigger: "axis", ...tooltipStyle },
      legend: { show: false, textStyle: { color: axisLabelColor } },
      grid: { left: 60, right: 20, top: 16, bottom: 40, containLabel: false },
      xAxis: { type: "category", data: x, name: spec.x, ...axis },
      yAxis: { type: "value", name: spec.y, splitLine, ...axis },
      series: [{
        name: spec.y,
        type: "bar",
        data: y,
        itemStyle: { borderRadius: [3, 3, 0, 0] },
      }],
    };
  }, [columns, rows, isDark]);

  if (!option) {
    const spec = autoChartAxes(columns);
    if (spec.type === "stat") {
      const v = rows.at(-1)?.[spec.y];
      return (
        <div className="pv-flex pv-h-full pv-flex-col pv-items-center pv-justify-center">
          <div className="pv-text-sm pv-text-muted-foreground">{spec.y}</div>
          <div className="pv-text-5xl pv-font-bold pv-tabular-nums">{String(v ?? "—")}</div>
        </div>
      );
    }
    // "none" state
    return (
      <div className="pv-flex pv-h-full pv-flex-col pv-items-center pv-justify-center pv-gap-3 pv-rounded pv-border pv-border-border/40 pv-bg-muted/20 pv-p-6">
        <BarChart3 className="pv-h-8 pv-w-8 pv-text-muted-foreground" />
        <p className="pv-text-sm pv-text-muted-foreground">No auto-chart for this shape.</p>
      </div>
    );
  }

  return <ReactECharts option={option} style={{ height: "100%", width: "100%" }} theme="default" notMerge={true} />;
}
