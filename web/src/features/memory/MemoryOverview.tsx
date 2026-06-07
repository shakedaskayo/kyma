import { motion } from "framer-motion";
import { Activity, Workflow } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { useReducedMotion } from "@/lib/motion";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { SkeletonRows } from "@/components/ui/skeleton";
import { NodesStrip } from "@/features/dreaming/NodesStrip";
import type { PipelineRun } from "@/sdk/memory";
import { DreamingStatusCard } from "./DreamingStatusCard";
import { LivePulse } from "./LivePulse";
import { StoreAtAGlance } from "./StoreAtAGlance";
import { useMemoryOverview } from "./useMemoryOverview";

/**
 * The Memory Overview — the landing showpiece. Asymmetric two-column layout on
 * desktop: a main content column (dreaming status, the store at a glance, recent
 * pipeline runs, nodes) and a live rail (the streaming firehose pulse). Stacks
 * gracefully on narrow viewports.
 */
export function MemoryOverview() {
  const reduce = useReducedMotion();
  const { data, isLoading } = useMemoryOverview();

  const fadeUp = (delay: number) =>
    reduce
      ? {}
      : {
          initial: { opacity: 0, y: 10 },
          animate: { opacity: 1, y: 0 },
          transition: { duration: 0.32, ease: [0.16, 1, 0.3, 1] as const, delay },
        };

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto w-full max-w-6xl px-6 py-6">
        <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_20rem] xl:grid-cols-[minmax(0,1fr)_22rem]">
          {/* MAIN COLUMN */}
          <div className="min-w-0 space-y-6">
            <motion.div {...fadeUp(0)}>
              <DreamingStatusCard />
            </motion.div>

            <motion.div {...fadeUp(0.06)}>
              {isLoading && !data ? (
                <div className="rounded-xl border border-border/60 bg-card/40 p-5">
                  <SkeletonRows rows={4} />
                </div>
              ) : (
                <StoreAtAGlance
                  counts={data?.memory.counts ?? []}
                  recent={data?.memory.recent ?? []}
                />
              )}
            </motion.div>

            <motion.div {...fadeUp(0.12)}>
              <PipelineRuns runs={data?.pipeline_runs ?? []} />
            </motion.div>

            <motion.div {...fadeUp(0.18)}>
              <div className="rounded-xl border border-border/60 bg-card/40 px-1 pt-1">
                {/* NodesStrip carries its own border-b chrome; wrap it so it sits
                    in the designed grid rather than as a full-bleed strip. */}
                <div className="[&>div]:border-0 [&>div]:bg-transparent">
                  <NodesStrip />
                </div>
              </div>
            </motion.div>
          </div>

          {/* LIVE RAIL */}
          <motion.aside
            {...fadeUp(0.1)}
            className="lg:sticky lg:top-0 lg:h-[calc(100vh-7.5rem)]"
          >
            <div className="flex h-full min-h-[24rem] flex-col rounded-xl border border-border/60 bg-surface/60 p-4 shadow-elev-1">
              <LivePulse className="min-h-0 flex-1" />
            </div>
          </motion.aside>
        </div>
      </div>
    </div>
  );
}

/** Recent pipeline runs (consolidation + dreaming) as a slim timeline strip. */
function PipelineRuns({ runs }: { runs: PipelineRun[] }) {
  if (runs.length === 0) return null;
  return (
    <section className="rounded-xl border border-border/60 bg-card/40 p-5 shadow-elev-1">
      <header className="mb-3 flex items-center gap-2">
        <Workflow className="h-3.5 w-3.5 text-primary/80" />
        <h2 className="text-2xs font-medium uppercase tracking-[0.14em] text-foreground/80">
          Pipeline
        </h2>
        <Link
          to="/memory/dreaming"
          className="ml-auto text-2xs text-muted-foreground transition-colors hover:text-foreground"
        >
          View runs
        </Link>
      </header>
      <ul className="space-y-1.5">
        {runs.slice(0, 6).map((r) => (
          <li
            key={r.id}
            className="flex items-center gap-3 rounded-md px-2 py-1.5 text-xs transition-colors hover:bg-card/70"
          >
            <span
              className={cn(
                "h-1.5 w-1.5 shrink-0 rounded-full",
                r.status === "success"
                  ? "bg-emerald-400"
                  : r.status === "error"
                    ? "bg-rose-400"
                    : r.status === "running"
                      ? "bg-violet-400 animate-pulse"
                      : "bg-muted-foreground",
              )}
            />
            <span className="w-28 shrink-0 capitalize text-foreground/85">{r.kind}</span>
            <span className="flex items-center gap-1 text-muted-foreground">
              <Activity className="h-3 w-3" />
              <span className="tabular-nums">{r.events_scanned}</span> scanned
            </span>
            <span className="text-muted-foreground">
              <span className="tabular-nums text-foreground/80">{r.memories_written}</span> written
            </span>
            <span className="ml-auto shrink-0 tabular-nums text-muted-foreground">
              {relTime(r.finished_at ?? r.started_at)}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
