import { Link } from "@tanstack/react-router";
import { motion } from "framer-motion";
import { ArrowUpRight, Database } from "lucide-react";
import { useReducedMotion } from "@/lib/motion";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { Row } from "@/sdk/memory";
import { AnimatedCounter } from "./AnimatedCounter";
import { MEMORY_TYPES, num, str, typeStyle } from "./lib";

/**
 * "The memory store at a glance" — a single composed segmented bar of the type
 * distribution, a legend, and a refined recent-memories list with importance
 * indicators. Reads as one elegant block, not stacked cards.
 */
export function StoreAtAGlance({
  counts,
  recent,
}: {
  counts: Row[];
  recent: Row[];
}) {
  const reduce = useReducedMotion();

  // Aggregate counts by memory type (overview returns one row per
  // type×status×realm).
  const byType = new Map<string, number>();
  for (const r of counts) {
    const t = str(r.memory_type) || "summary";
    byType.set(t, (byType.get(t) ?? 0) + num(r.n));
  }
  const total = [...byType.values()].reduce((a, b) => a + b, 0);
  const ordered: { type: string; n: number }[] = MEMORY_TYPES.map((t) => ({
    type: t as string,
    n: byType.get(t) ?? 0,
  })).filter((x) => x.n > 0);
  // Include any unknown types the server returns.
  for (const [t, n] of byType) {
    if (!MEMORY_TYPES.includes(t as never)) ordered.push({ type: t, n });
  }

  return (
    <section className="rounded-xl border border-border/60 bg-card/40 p-5 shadow-elev-1">
      <header className="flex items-center gap-2">
        <Database className="h-3.5 w-3.5 text-primary/80" />
        <h2 className="text-2xs font-medium uppercase tracking-[0.14em] text-foreground/80">
          Memory store
        </h2>
        <span className="ml-auto flex items-baseline gap-1.5">
          <AnimatedCounter value={total} className="text-2xl font-semibold tracking-tight" />
          <span className="text-xs text-muted-foreground">memories</span>
        </span>
      </header>

      {/* Segmented distribution bar */}
      <div className="mt-4">
        {total === 0 ? (
          <div className="h-2.5 w-full rounded-full bg-muted/50" />
        ) : (
          <div className="flex h-2.5 w-full overflow-hidden rounded-full bg-muted/40">
            {ordered.map((x, i) => {
              const pct = (x.n / total) * 100;
              return (
                <motion.div
                  key={x.type}
                  initial={reduce ? false : { width: 0 }}
                  animate={{ width: `${pct}%` }}
                  transition={{ duration: 0.5, ease: [0.16, 1, 0.3, 1], delay: i * 0.04 }}
                  className={cn("h-full", typeStyle(x.type).dot)}
                  style={{ minWidth: pct > 0 ? 6 : 0 }}
                  title={`${x.type}: ${x.n}`}
                />
              );
            })}
          </div>
        )}

        {/* Legend */}
        <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1.5">
          {ordered.length === 0 ? (
            <span className="text-xs text-muted-foreground">No memories stored yet.</span>
          ) : (
            ordered.map((x) => (
              <span key={x.type} className="flex items-center gap-1.5 text-xs">
                <span className={cn("h-2 w-2 rounded-full", typeStyle(x.type).dot)} />
                <span className="text-foreground/80">{x.type}</span>
                <span className="font-medium tabular-nums text-muted-foreground">{x.n}</span>
              </span>
            ))
          )}
        </div>
      </div>

      {/* Recent memories */}
      {recent.length > 0 && (
        <div className="mt-5 border-t border-border/50 pt-4">
          <div className="mb-2.5 flex items-center justify-between">
            <span className="text-2xs font-medium uppercase tracking-[0.14em] text-muted-foreground">
              Recently distilled
            </span>
            <Link
              to="/memory/search"
              className="flex items-center gap-0.5 text-2xs text-muted-foreground transition-colors hover:text-foreground"
            >
              Search all <ArrowUpRight className="h-3 w-3" />
            </Link>
          </div>
          <ul className="space-y-2.5">
            {recent.slice(0, 5).map((r, i) => (
              <RecentRow key={str(r.id) || i} r={r} now={Date.now()} />
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

function RecentRow({ r, now }: { r: Row; now: number }) {
  const type = str(r.memory_type) || "summary";
  const st = typeStyle(type);
  const importance = num(r.importance);
  const created = str(r.created_at);
  return (
    <li className="group flex gap-3">
      {/* Importance gauge (4 ticks) */}
      <ImportanceGauge value={importance} className="mt-0.5" />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={cn("rounded px-1.5 py-0.5 text-[10px] font-medium", st.chip)}>
            {type}
          </span>
          {str(r.realm) && (
            <span className="text-[10px] text-muted-foreground">{str(r.realm)}</span>
          )}
          <span className="ml-auto shrink-0 text-[10px] tabular-nums text-muted-foreground">
            {created ? relTime(created, now) : ""}
          </span>
        </div>
        <p className="mt-1 line-clamp-2 text-xs leading-relaxed text-foreground/80">
          {str(r.content_preview)}
        </p>
      </div>
    </li>
  );
}

/** A vertical 4-segment importance gauge — fills bottom-up by importance 0–1. */
function ImportanceGauge({ value, className }: { value: number; className?: string }) {
  const filled = Math.round(Math.max(0, Math.min(1, value)) * 4);
  return (
    <span
      className={cn("flex h-7 w-1 flex-col-reverse gap-0.5", className)}
      title={`importance ${value.toFixed(2)}`}
      aria-label={`importance ${value.toFixed(2)}`}
    >
      {Array.from({ length: 4 }).map((_, i) => (
        <span
          key={i}
          className={cn(
            "flex-1 rounded-full",
            i < filled ? "bg-violet-400/80" : "bg-muted/60",
          )}
        />
      ))}
    </span>
  );
}
