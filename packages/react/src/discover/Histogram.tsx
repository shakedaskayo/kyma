import { useMemo, useRef, useState } from "react";
import type { DiscoverResultsState } from "./types";

const PALETTE = [
  "#3b82f6", "#10b981", "#f59e0b", "#ef4444",
  "#8b5cf6", "#ec4899", "#14b8a6", "#f97316",
];

type Props = {
  results: DiscoverResultsState;
  onZoom?: (fromIso: string, toIso: string) => void;
  rangeTo?: string | null;
};

export function Histogram({ results, onZoom, rangeTo }: Props) {
  const { bars, sourceColors } = useMemo(() => stack(results), [results]);
  const [drag, setDrag] = useState<{ start: number; end: number } | null>(null);
  const wrap = useRef<HTMLDivElement>(null);
  if (bars.length === 0) return null;

  const max = bars.reduce((m, b) => Math.max(m, b.total), 0) || 1;
  const fmt = (iso: string) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  };
  const idxAt = (clientX: number) => {
    const el = wrap.current;
    if (!el) return 0;
    const r = el.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    return Math.min(bars.length - 1, Math.floor(frac * bars.length));
  };
  const finishDrag = () => {
    if (!drag || !onZoom) return setDrag(null);
    const [lo, hi] = [Math.min(drag.start, drag.end), Math.max(drag.start, drag.end)];
    if (hi > lo) {
      const from = bars[lo].label;
      const to = bars[hi + 1]?.label ?? rangeTo ?? new Date().toISOString();
      onZoom(from, to);
    }
    setDrag(null);
  };
  const inDrag = (i: number) =>
    drag != null && i >= Math.min(drag.start, drag.end) && i <= Math.max(drag.start, drag.end);

  return (
    <div className="pv-border-b pv-select-none">
      <div
        ref={wrap}
        className="pv-flex pv-items-end pv-h-24 pv-px-2 pv-gap-px pv-cursor-crosshair"
        onMouseDown={(e) => setDrag({ start: idxAt(e.clientX), end: idxAt(e.clientX) })}
        onMouseMove={(e) => drag && setDrag({ ...drag, end: idxAt(e.clientX) })}
        onMouseUp={finishDrag}
        onMouseLeave={() => setDrag(null)}
      >
        {bars.map((b, i) => (
          <div
            key={i}
            className={`pv-flex-1 pv-flex pv-flex-col-reverse ${inDrag(i) ? "pv-bg-accent" : ""}`}
            title={`${fmt(b.label)} — ${b.total} events`}
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
      <div className="pv-flex pv-justify-between pv-px-2 pv-text-[10px] pv-text-muted-foreground pv-tabular-nums">
        <span>{fmt(bars[0].label)}</span>
        {bars.length > 2 && <span>{fmt(bars[Math.floor(bars.length / 2)].label)}</span>}
        <span>{fmt(bars[bars.length - 1].label)}</span>
      </div>
      <div className="pv-flex pv-gap-3 pv-px-2 pv-py-1 pv-text-[10px] pv-text-muted-foreground pv-flex-wrap">
        {Array.from(sourceColors.entries()).map(([src, idx]) => (
          <span key={src} className="pv-inline-flex pv-items-center pv-gap-1">
            <span
              className="pv-inline-block pv-size-2 pv-rounded-sm"
              style={{ backgroundColor: PALETTE[idx % PALETTE.length] }}
            />
            <span className="pv-font-mono">{src}</span>
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
