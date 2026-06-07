import { useMemo } from "react";
import { cn } from "@/lib/utils";

/**
 * A compact inline sparkline (area + last-point dot) for the header's
 * events-per-hour signal. Pure SVG, no deps. `live` tints the last point violet
 * and gives it a soft glow so the header reads as "breathing".
 */
export function Sparkline({
  values,
  width = 96,
  height = 24,
  className,
  live = false,
}: {
  values: number[];
  width?: number;
  height?: number;
  className?: string;
  live?: boolean;
}) {
  const { line, area, lastX, lastY, hasData } = useMemo(() => {
    const vs = values.length > 0 ? values : [0];
    const max = Math.max(1, ...vs);
    const n = vs.length;
    const stepX = n > 1 ? width / (n - 1) : width;
    const pad = 2;
    const usableH = height - pad * 2;
    const pts = vs.map((v, i) => {
      const x = n > 1 ? i * stepX : width / 2;
      const y = pad + usableH - (v / max) * usableH;
      return [x, y] as const;
    });
    const lineD = pts.map((p, i) => `${i === 0 ? "M" : "L"}${p[0].toFixed(1)},${p[1].toFixed(1)}`).join(" ");
    const areaD = `${lineD} L${width},${height} L0,${height} Z`;
    const last = pts[pts.length - 1];
    return {
      line: lineD,
      area: areaD,
      lastX: last[0],
      lastY: last[1],
      hasData: values.some((v) => v > 0),
    };
  }, [values, width, height]);

  const stroke = live ? "hsl(258 90% 70%)" : "hsl(var(--primary))";

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      className={cn("overflow-visible", className)}
      aria-hidden
    >
      <defs>
        <linearGradient id="spark-fill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={stroke} stopOpacity="0.22" />
          <stop offset="100%" stopColor={stroke} stopOpacity="0" />
        </linearGradient>
      </defs>
      {hasData && <path d={area} fill="url(#spark-fill)" />}
      <path d={line} fill="none" stroke={stroke} strokeWidth="1.5" strokeLinejoin="round" strokeLinecap="round" opacity={hasData ? 1 : 0.4} />
      {hasData && (
        <circle cx={lastX} cy={lastY} r={live ? 2.4 : 1.8} fill={stroke}>
          {live && (
            <animate attributeName="r" values="2.2;3.4;2.2" dur="1.8s" repeatCount="indefinite" />
          )}
        </circle>
      )}
    </svg>
  );
}
