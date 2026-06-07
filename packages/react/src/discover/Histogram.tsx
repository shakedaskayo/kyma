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
    <div className="ky-border-b ky-select-none">
      <div
        ref={wrap}
        className="ky-flex ky-items-end ky-h-24 ky-px-2 ky-gap-px ky-cursor-crosshair"
        onMouseDown={(e) => setDrag({ start: idxAt(e.clientX), end: idxAt(e.clientX) })}
        onMouseMove={(e) => drag && setDrag({ ...drag, end: idxAt(e.clientX) })}
        onMouseUp={finishDrag}
        onMouseLeave={() => setDrag(null)}
      >
        {bars.map((b, i) => (
          <div
            key={i}
            className={`ky-flex-1 ky-flex ky-flex-col-reverse ${inDrag(i) ? "ky-bg-accent" : ""}`}
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
      <div className="ky-flex ky-justify-between ky-px-2 ky-text-[10px] ky-text-muted-foreground ky-tabular-nums">
        <span>{fmt(bars[0].label)}</span>
        {bars.length > 2 && <span>{fmt(bars[Math.floor(bars.length / 2)].label)}</span>}
        <span>{fmt(bars[bars.length - 1].label)}</span>
      </div>
      <div className="ky-flex ky-gap-3 ky-px-2 ky-py-1 ky-text-[10px] ky-text-muted-foreground ky-flex-wrap">
        {Array.from(sourceColors.entries()).map(([src, idx]) => (
          <span key={src} className="ky-inline-flex ky-items-center ky-gap-1">
            <span
              className="ky-inline-block ky-size-2 ky-rounded-sm"
              style={{ backgroundColor: PALETTE[idx % PALETTE.length] }}
            />
            <span className="ky-font-mono">{src}</span>
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
