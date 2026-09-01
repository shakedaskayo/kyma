import ReactECharts from "echarts-for-react";
import { useMemo } from "react";
import { pickBucketSize, BUCKET_MS, bucketLabel } from "./time-bucket";
import { usePensieveContext } from "../provider/context";

export type HistogramTimelineProps = {
  rows: Record<string, unknown>[];
  timeCol: string;
  onBucketClick?: (from: Date, to: Date) => void;
};

export function HistogramTimeline({ rows, timeCol, onBucketClick }: HistogramTimelineProps) {
  const { isDark } = usePensieveContext();
  const { buckets, size } = useMemo(() => {
    const times: number[] = [];
    for (const row of rows) {
      const v = row[timeCol];
      if (!v) continue;
      const t = typeof v === "string" ? new Date(v).getTime() : typeof v === "number" ? v : NaN;
      if (!isNaN(t)) times.push(t);
    }
    if (times.length === 0) return { buckets: [], size: "1h" as const };

    const minT = Math.min(...times);
    const maxT = Math.max(...times);
    const spanMs = maxT - minT;
    const bSize = pickBucketSize(spanMs);
    const bMs = BUCKET_MS[bSize];

    const map = new Map<number, number>();
    for (const t of times) {
      const key = Math.floor(t / bMs) * bMs;
      map.set(key, (map.get(key) ?? 0) + 1);
    }

    const firstKey = Math.floor(minT / bMs) * bMs;
    const lastKey  = Math.floor(maxT / bMs) * bMs;
    const filled: Array<{ ts: number; count: number }> = [];
    for (let k = firstKey; k <= lastKey; k += bMs) {
      filled.push({ ts: k, count: map.get(k) ?? 0 });
    }

    return { buckets: filled, size: bSize };
  }, [rows, timeCol]);

  if (buckets.length === 0) return null;

  const labels = buckets.map((b) => bucketLabel(b.ts, size));
  const values = buckets.map((b) => b.count);

  const splitLineColor = isDark ? "#1e293b" : "#f1f5f9";
  const tooltipBg = isDark ? "rgba(15,23,42,0.95)" : "rgba(255,255,255,0.95)";
  const tooltipText = isDark ? "#e2e8f0" : "#0f172a";
  const tooltipBorder = isDark ? "#1e293b" : "#e2e8f0";

  const option = {
    animation: false,
    grid: { left: 48, right: 8, top: 6, bottom: 22 },
    xAxis: {
      type: "category",
      data: labels,
      axisLabel: { fontSize: 10, color: "#94a3b8", interval: Math.max(0, Math.floor(buckets.length / 8) - 1) },
      axisLine: { show: false },
      axisTick: { show: false },
    },
    yAxis: {
      type: "value",
      axisLabel: { fontSize: 9, color: "#94a3b8" },
      splitLine: { lineStyle: { color: splitLineColor } },
      minInterval: 1,
    },
    series: [
      {
        type: "bar",
        data: values,
        barMaxWidth: 32,
        itemStyle: { color: "rgba(59,130,246,0.7)", borderRadius: [2, 2, 0, 0] },
        emphasis: { itemStyle: { color: "rgba(59,130,246,0.95)" } },
      },
    ],
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "shadow" },
      backgroundColor: tooltipBg,
      borderColor: tooltipBorder,
      textStyle: { fontSize: 11, color: tooltipText },
      formatter: (params: Array<{ name: string; value: number }>) => {
        const p = params[0];
        return `${p.name}<br/><b>${p.value.toLocaleString()}</b> events`;
      },
    },
  };

  const handleClick = (params: { dataIndex?: number }) => {
    if (!onBucketClick || params.dataIndex === undefined) return;
    const b = buckets[params.dataIndex];
    if (!b) return;
    const bMs = BUCKET_MS[size];
    onBucketClick(new Date(b.ts), new Date(b.ts + bMs));
  };

  return (
    <div className="pv-bg-card pv-border-b pv-px-2">
      <ReactECharts
        option={option}
        style={{ height: 80 }}
        onEvents={{ click: handleClick }}
        opts={{ renderer: "svg" }}
      />
    </div>
  );
}
