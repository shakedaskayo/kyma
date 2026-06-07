import { useMemo } from "react";
import { Link, useRouterState } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Brain, LayoutGrid, Moon, Search, SlidersHorizontal } from "lucide-react";
import { useReducedMotion } from "@/lib/motion";
import { cn } from "@/lib/utils";
import { useSession } from "@/sdk/session";
import { getMemorySettings, type DreamingSettings } from "@/sdk/memory";
import { useDreamingRuns } from "@/features/dreaming/useDreaming";
import { AnimatedCounter } from "./AnimatedCounter";
import { Sparkline } from "./Sparkline";
import { useMemoryOverview } from "./useMemoryOverview";
import { useSharedMemoryStream } from "./stream-context";
import { intervalLabel, num } from "./lib";

const TABS = [
  { to: "/memory/overview", label: "Overview", icon: LayoutGrid, match: "/memory/overview" },
  { to: "/memory/search", label: "Search", icon: Search, match: "/memory/search" },
  { to: "/memory/dreaming", label: "Dreaming", icon: Moon, match: "/memory/dreaming" },
  { to: "/memory/settings", label: "Settings", icon: SlidersHorizontal, match: "/memory/settings" },
] as const;

/**
 * The slim sticky page header for the whole Memory surface. Carries the title,
 * the engine's live status (dreaming state, store size, an events-per-hour
 * sparkline + a live events/min readout), and the segmented section nav.
 */
export function MemoryHeader() {
  const reduce = useReducedMotion();
  const { endpoint, token } = useSession();
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  const { data: overview } = useMemoryOverview();
  const { eventsPerMin, status } = useSharedMemoryStream();

  const { data: settings } = useQuery({
    queryKey: ["memory-settings", endpoint],
    queryFn: () => getMemorySettings({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    staleTime: 30_000,
  });
  const dreaming: DreamingSettings | undefined = settings?.dreaming;

  const { data: runs } = useDreamingRuns({ limit: 3 });
  const running = Boolean(runs?.some((r) => r.status === "running"));

  const storeSize = useMemo(
    () => (overview?.memory.counts ?? []).reduce((a, r) => a + num(r.n), 0),
    [overview],
  );

  const spark = useMemo(
    () => (overview?.firehose.timeline ?? []).slice(-24).map((r) => num(r.n)),
    [overview],
  );

  const live = status === "live";

  const engineState: "dreaming" | "armed" | "asleep" = running
    ? "dreaming"
    : dreaming?.enabled
      ? "armed"
      : "asleep";

  return (
    <header className="sticky top-0 z-20 border-b border-border/60 bg-background/85 backdrop-blur-md">
      {/* Faint brand glow on the header. */}
      <div className="pointer-events-none absolute inset-0 bg-[radial-gradient(40rem_12rem_at_8%_-60%,hsl(258_90%_66%/0.08),transparent_70%)]" />
      <div className="relative mx-auto w-full max-w-6xl px-6">
        {/* Top row: title + live engine status */}
        <div className="flex flex-wrap items-center gap-x-5 gap-y-2 pb-2.5 pt-3">
          <div className="flex items-center gap-2.5">
            <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-violet-500/12 text-violet-300 ring-1 ring-inset ring-violet-400/20">
              <Brain className="h-4 w-4" />
            </span>
            <div className="leading-tight">
              <h1 className="text-base font-semibold tracking-tight">Memory</h1>
              <p className="text-2xs text-muted-foreground">Distributed context engine</p>
            </div>
          </div>

          <div className="ml-auto flex flex-wrap items-center gap-x-5 gap-y-1.5">
            <EngineStat label="Engine">
              <span className="flex items-center gap-1.5">
                <span className="relative flex h-1.5 w-1.5">
                  {engineState === "dreaming" && !reduce && (
                    <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-violet-500/70" />
                  )}
                  <span
                    className={cn(
                      "relative inline-flex h-1.5 w-1.5 rounded-full",
                      engineState === "dreaming"
                        ? "bg-violet-400"
                        : engineState === "armed"
                          ? "bg-emerald-400"
                          : "bg-muted-foreground",
                    )}
                  />
                </span>
                <span
                  className={cn(
                    "text-xs font-medium",
                    engineState === "dreaming"
                      ? "text-violet-300"
                      : engineState === "armed"
                        ? "text-emerald-300/90"
                        : "text-muted-foreground",
                  )}
                >
                  {engineState === "dreaming"
                    ? "Dreaming"
                    : engineState === "armed"
                      ? `Armed · ${dreaming ? intervalLabel(dreaming.interval_secs) : ""}`
                      : "Asleep"}
                </span>
              </span>
            </EngineStat>

            <span className="hidden h-7 w-px bg-border/70 sm:block" />

            <EngineStat label="Store">
              <AnimatedCounter value={storeSize} className="text-xs font-medium text-foreground/90" />
            </EngineStat>

            <span className="hidden h-7 w-px bg-border/70 sm:block" />

            <EngineStat label={live ? "Events · live" : "Events / hr"}>
              <span className="flex items-center gap-2">
                <Sparkline values={spark} live={live} />
                {eventsPerMin > 0 && (
                  <span className="text-2xs tabular-nums text-muted-foreground">
                    {eventsPerMin}/min
                  </span>
                )}
              </span>
            </EngineStat>
          </div>
        </div>

        {/* Bottom row: section nav */}
        <nav className="flex items-center gap-1 pb-1.5">
          {TABS.map((t) => {
            const active = pathname.startsWith(t.match);
            const Icon = t.icon;
            return (
              <Link
                key={t.to}
                to={t.to}
                className={cn(
                  "relative flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  active
                    ? "text-foreground"
                    : "text-muted-foreground hover:text-foreground",
                )}
              >
                <Icon className="h-3.5 w-3.5" />
                {t.label}
                {active && (
                  <span className="absolute inset-x-2 -bottom-[7px] h-0.5 rounded-full bg-violet-400" />
                )}
              </Link>
            );
          })}
        </nav>
      </div>
    </header>
  );
}

function EngineStat({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-[9px] font-medium uppercase tracking-[0.16em] text-muted-foreground/80">
        {label}
      </span>
      {children}
    </div>
  );
}
