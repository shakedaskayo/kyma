import { useMemo } from "react";
import type { DiscoverResultsState } from "./types";

const PALETTE = [
  "#3b82f6", "#10b981", "#f59e0b", "#ef4444",
  "#8b5cf6", "#ec4899", "#14b8a6", "#f97316",
];

type Props = { results: DiscoverResultsState };

export function Histogram({ results }: Props) {
  const { bars, sourceColors } = useMemo(() => stack(results), [results]);
  if (bars.length === 0) return null;

  const max = bars.reduce((m, b) => Math.max(m, b.total), 0) || 1;

  return (
    <div className="border-b">
      <div className="flex items-end h-24 px-2 gap-px">
        {bars.map((b, i) => (
          <div
            key={i}
            className="flex-1 flex flex-col-reverse"
            title={`${b.label} — ${b.total}`}
          >
            {b.segments.map((seg) => (
              <div
                key={seg.source}
                style={{
                  height: `${(seg.n / max) * 100}%`,
                  backgroundColor: PALETTE[seg.colorIdx % PALETTE.length],
                }}
              />
            ))}
          </div>
        ))}
      </div>
      <div className="flex gap-3 px-2 py-1 text-[10px] text-muted-foreground flex-wrap">
        {Array.from(sourceColors.entries()).map(([src, idx]) => (
          <span key={src} className="inline-flex items-center gap-1">
            <span
              className="inline-block size-2 rounded-sm"
              style={{ backgroundColor: PALETTE[idx % PALETTE.length] }}
            />
            <span className="font-mono">{src}</span>
          </span>
        ))}
      </div>
    </div>
  );
}

function stack(r: DiscoverResultsState) {
  const sourceColors = new Map<string, number>();
  Array.from(r.sources.keys()).forEach((s, i) => sourceColors.set(s, i));

  const bucketIndex = new Map<string, Map<string, number>>();
  for (const [src, st] of r.sources) {
    if (!st.histogram) continue;
    for (const b of st.histogram) {
      const m = bucketIndex.get(b.t) ?? new Map<string, number>();
      m.set(src, (m.get(src) ?? 0) + b.n);
      bucketIndex.set(b.t, m);
    }
  }

  const ordered = Array.from(bucketIndex.entries()).sort(([a], [b]) => a.localeCompare(b));
  const bars = ordered.map(([t, m]) => {
    const segs = Array.from(sourceColors.entries()).map(([src, idx]) => ({
      source: src,
      n: m.get(src) ?? 0,
      colorIdx: idx,
    }));
    const total = segs.reduce((s, v) => s + v.n, 0);
    return { label: t, total, segments: segs };
  });

  return { bars, sourceColors };
}
