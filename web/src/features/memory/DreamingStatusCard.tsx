import { Link } from "@tanstack/react-router";
import { motion } from "framer-motion";
import { ArrowUpRight, Moon, Settings2, Sparkles } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { useReducedMotion } from "@/lib/motion";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { useSession } from "@/sdk/session";
import { getMemorySettings, type DreamingSettings } from "@/sdk/memory";
import { useDreamingRuns, useTriggerDreaming } from "@/features/dreaming/useDreaming";
import { intervalLabel } from "./lib";

/**
 * Dreaming status block for the Overview: current engine state (asleep / armed /
 * dreaming), the last run outcome, and a primary "Dream now" action. While a run
 * is in flight the card breathes (slow violet glow) and shows the live phase.
 */
export function DreamingStatusCard({ className }: { className?: string }) {
  const { endpoint, token } = useSession();
  const reduce = useReducedMotion();

  const { data: settings } = useQuery({
    queryKey: ["memory-settings", endpoint],
    queryFn: () => getMemorySettings({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    staleTime: 30_000,
  });
  const dreaming: DreamingSettings | undefined = settings?.dreaming;

  const { data: runs } = useDreamingRuns({ limit: 5 });
  const trigger = useTriggerDreaming();

  const running = runs?.find((r) => r.status === "running") ?? null;
  const lastSettled = runs?.find((r) => r.status !== "running") ?? null;
  const enabled = dreaming?.enabled ?? false;

  const state: "dreaming" | "armed" | "asleep" = running
    ? "dreaming"
    : enabled
      ? "armed"
      : "asleep";

  return (
    <motion.section
      animate={
        running && !reduce
          ? { boxShadow: ["0 0 0 1px hsl(258 90% 66% / 0.25)", "0 0 22px hsl(258 90% 66% / 0.35)", "0 0 0 1px hsl(258 90% 66% / 0.25)"] }
          : {}
      }
      transition={running && !reduce ? { duration: 2.6, repeat: Infinity, ease: "easeInOut" } : {}}
      className={cn(
        "relative overflow-hidden rounded-xl border border-border/60 p-5",
        running
          ? "bg-gradient-to-br from-violet-500/[0.10] via-card/40 to-card/40"
          : "bg-card/40 shadow-elev-1",
        className,
      )}
    >
      <header className="flex items-center gap-2">
        <Moon className="h-3.5 w-3.5 text-violet-400/90" />
        <h2 className="text-2xs font-medium uppercase tracking-[0.14em] text-foreground/80">
          Dreaming
        </h2>
        <Link
          to="/memory/dreaming"
          className="ml-auto flex items-center gap-0.5 text-2xs text-muted-foreground transition-colors hover:text-foreground"
        >
          All runs <ArrowUpRight className="h-3 w-3" />
        </Link>
      </header>

      <div className="mt-3 flex items-start gap-4">
        {/* Orb */}
        <div className="relative mt-0.5 shrink-0">
          {running && !reduce && (
            <span className="absolute inset-0 -m-1 animate-ping rounded-full bg-violet-500/25" />
          )}
          <span
            className={cn(
              "relative flex h-11 w-11 items-center justify-center rounded-full ring-1 ring-inset",
              state === "dreaming"
                ? "bg-violet-500/15 text-violet-300 ring-violet-400/30"
                : state === "armed"
                  ? "bg-violet-500/8 text-violet-300/80 ring-violet-400/20"
                  : "bg-muted/40 text-muted-foreground ring-border/60",
            )}
          >
            <Moon className="h-5 w-5" />
          </span>
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="text-base font-semibold tracking-tight">
              {state === "dreaming" ? "Dreaming…" : state === "armed" ? "Armed" : "Asleep"}
            </span>
            {state === "armed" && dreaming && (
              <span className="text-2xs text-muted-foreground">
                every {intervalLabel(dreaming.interval_secs)} · {dreaming.mode.replace(/_/g, " ")}
              </span>
            )}
          </div>

          {running ? (
            <p className="mt-1 truncate text-xs text-violet-300/90">
              {running.progress?.current_phase ?? "Consolidating memory…"}
            </p>
          ) : lastSettled ? (
            <p className="mt-1 text-xs text-muted-foreground">
              Last run {relTime(lastSettled.finished_at ?? lastSettled.started_at)} ·{" "}
              <span
                className={cn(
                  "capitalize",
                  lastSettled.status === "success"
                    ? "text-emerald-400/90"
                    : lastSettled.status === "error"
                      ? "text-rose-400/90"
                      : "text-muted-foreground",
                )}
              >
                {lastSettled.status}
              </span>
              {lastSettled.stats?.summary ? ` — ${truncate(lastSettled.stats.summary, 70)}` : ""}
            </p>
          ) : (
            <p className="mt-1 text-xs text-muted-foreground">
              The engine consolidates, merges, and links memory autonomously.
            </p>
          )}
        </div>
      </div>

      <div className="mt-4 flex items-center gap-2">
        <Button
          size="sm"
          onClick={() => trigger.mutate(undefined)}
          disabled={Boolean(running) || trigger.isPending}
          className={cn(
            "gap-1.5",
            !running && "bg-violet-600 text-white hover:bg-violet-500",
          )}
        >
          <Sparkles className="h-3.5 w-3.5" />
          {running ? "Dreaming…" : "Dream now"}
        </Button>
        <Button size="sm" variant="ghost" asChild>
          <Link to="/memory/settings" aria-label="Dreaming settings">
            <Settings2 className="mr-1 h-3.5 w-3.5" /> Configure
          </Link>
        </Button>
      </div>

      {/* Indeterminate progress while running */}
      {running && (
        <div className="absolute inset-x-0 bottom-0 h-0.5 overflow-hidden bg-violet-500/10">
          <div className="h-full w-1/3 animate-shimmer bg-gradient-to-r from-transparent via-violet-500 to-transparent" />
        </div>
      )}
    </motion.section>
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}
