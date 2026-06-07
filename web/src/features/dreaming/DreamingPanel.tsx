import { useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import { Moon, Settings2, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { StatusPill } from "@/components/ui/status";
import { useSession } from "@/sdk/session";
import { getMemorySettings, type DreamingSettings } from "@/sdk/memory";
import { useQuery } from "@tanstack/react-query";
import type { RunKind, RunStatus } from "@/sdk/dreaming";
import { cn } from "@/lib/utils";
import { relTime } from "@/lib/time";
import { intervalLabel } from "@/features/memory/lib";
import { useDreamingRuns, useTriggerDreaming } from "./useDreaming";
import { NodesStrip } from "./NodesStrip";
import { RunRow } from "./RunRow";
import { ActivityView } from "./ActivityView";

type SubTab = "runs" | "activity";
type KindFilter = "all" | RunKind;
type StatusFilter = "all" | RunStatus;

const PAGE = 20;

export function DreamingPanel() {
  const { endpoint, token } = useSession();
  const [subTab, setSubTab] = useState<SubTab>("runs");
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [limit, setLimit] = useState(PAGE);

  // Dreaming settings (enabled / mode / interval) drive the action strip + empty state.
  const { data: settings } = useQuery({
    queryKey: ["memory-settings", endpoint],
    queryFn: () => getMemorySettings({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    staleTime: 30_000,
  });
  const dreaming: DreamingSettings | undefined = settings?.dreaming;

  const kindArg = kindFilter === "all" ? undefined : kindFilter;
  const { data: runs, isLoading, error } = useDreamingRuns({ kind: kindArg, limit });
  const trigger = useTriggerDreaming();

  const filtered = useMemo(
    () => (runs ?? []).filter((r) => statusFilter === "all" || r.status === statusFilter),
    [runs, statusFilter],
  );

  const running = (runs ?? []).find((r) => r.status === "running") ?? null;
  const hasMore = (runs?.length ?? 0) >= limit;
  const isRunning = Boolean(running);

  return (
    <div className="flex h-full flex-col">
      {/* Compact action strip — the page header already carries engine status. */}
      <div className="border-b border-border/60 bg-surface/40">
        <div className="mx-auto flex w-full max-w-3xl flex-wrap items-center gap-3 px-6 py-3">
          <div className="flex items-center gap-2">
            {isRunning ? (
              <StatusPill tone="running" pulse>
                Dreaming now
              </StatusPill>
            ) : dreaming?.enabled ? (
              <StatusPill tone="ok">
                Armed · every {intervalLabel(dreaming.interval_secs)}
              </StatusPill>
            ) : (
              <StatusPill tone="idle">Asleep</StatusPill>
            )}
            {running?.progress?.current_phase && (
              <span className="truncate text-xs text-violet-300/90">
                {running.progress.current_phase}
              </span>
            )}
          </div>
          <div className="ml-auto flex items-center gap-2">
            <Button
              size="sm"
              onClick={() => trigger.mutate(undefined)}
              disabled={isRunning || trigger.isPending}
              className={cn(!isRunning && "bg-violet-600 text-white hover:bg-violet-500")}
            >
              <Sparkles className="mr-1.5 h-3.5 w-3.5" />
              {isRunning ? "Dreaming…" : "Dream now"}
            </Button>
            <Button variant="ghost" size="sm" asChild>
              <Link to="/memory/settings" aria-label="Dreaming settings">
                <Settings2 className="mr-1 h-3.5 w-3.5" /> Settings
              </Link>
            </Button>
          </div>
        </div>
      </div>

      {/* NODES */}
      <NodesStrip />

      {/* SUB-TABS */}
      <div className="border-b border-border/60">
        <div className="mx-auto flex w-full max-w-3xl items-center gap-1 px-6 py-2">
          <Segmented active={subTab === "runs"} onClick={() => setSubTab("runs")}>
            Runs
          </Segmented>
          <Segmented active={subTab === "activity"} onClick={() => setSubTab("activity")}>
            Activity
          </Segmented>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {subTab === "activity" ? (
          <ActivityView />
        ) : (
          <div className="mx-auto max-w-3xl space-y-3 px-6 py-4">
            {/* Filters */}
            <div className="flex flex-wrap items-center gap-1.5">
              <FilterGroup label="Kind">
                <Chip active={kindFilter === "all"} onClick={() => setKindFilter("all")}>
                  All
                </Chip>
                <Chip active={kindFilter === "dreaming"} onClick={() => setKindFilter("dreaming")}>
                  Dreaming
                </Chip>
                <Chip
                  active={kindFilter === "consolidation"}
                  onClick={() => setKindFilter("consolidation")}
                >
                  Consolidation
                </Chip>
              </FilterGroup>
              <span className="mx-1 h-4 w-px bg-border" />
              <FilterGroup label="Status">
                {(["all", "running", "success", "error"] as const).map((s) => (
                  <Chip key={s} active={statusFilter === s} onClick={() => setStatusFilter(s)}>
                    {s === "all" ? "All" : s[0].toUpperCase() + s.slice(1)}
                  </Chip>
                ))}
              </FilterGroup>
            </div>

            {/* Timeline */}
            {isLoading && !runs ? (
              <SkeletonRows rows={5} />
            ) : error ? (
              <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                Failed to load runs: {(error as Error).message}
              </div>
            ) : filtered.length === 0 ? (
              <RunsEmptyState
                dreaming={dreaming}
                onDream={() => trigger.mutate(undefined)}
                triggering={trigger.isPending}
              />
            ) : (
              <>
                <ol className="relative space-y-2.5 pl-5">
                  {/* The timeline spine. */}
                  <span className="absolute left-[5px] top-1 bottom-1 w-px bg-border/60" aria-hidden />
                  {filtered.map((r) => (
                    <li key={r.id} className="relative">
                      <span
                        className={cn(
                          "absolute -left-[18px] top-3.5 h-2 w-2 rounded-full ring-4 ring-surface/30",
                          r.status === "running"
                            ? "bg-violet-400 animate-pulse"
                            : r.status === "success"
                              ? "bg-emerald-400"
                              : r.status === "error"
                                ? "bg-rose-400"
                                : "bg-muted-foreground",
                        )}
                        aria-hidden
                      />
                      <RunRow run={r} />
                    </li>
                  ))}
                </ol>
                {hasMore && (
                  <div className="flex justify-center pt-1">
                    <Button variant="outline" size="sm" onClick={() => setLimit((l) => l + PAGE)}>
                      Load more
                    </Button>
                  </div>
                )}
                {filtered.some((r) => r.finished_at) && (
                  <p className="pt-1 text-center text-2xs text-muted-foreground">
                    {filtered.length} run{filtered.length === 1 ? "" : "s"} ·{" "}
                    {(() => {
                      const last = filtered.find((r) => r.finished_at);
                      return last ? `latest ${relTime(last.finished_at)}` : "";
                    })()}
                  </p>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function RunsEmptyState({
  dreaming,
  onDream,
  triggering,
}: {
  dreaming: DreamingSettings | undefined;
  onDream: () => void;
  triggering: boolean;
}) {
  if (dreaming && !dreaming.enabled) {
    return (
      <EmptyState
        icon={Moon}
        title="The engine is asleep"
        description="Enable autonomous consolidation to let the agent distill, merge, and link memories on a schedule — or wake it once now."
        action={
          <div className="flex items-center gap-2">
            <Button onClick={onDream} disabled={triggering} className="bg-violet-600 text-white hover:bg-violet-500">
              <Sparkles className="mr-1.5 h-4 w-4" /> Dream now
            </Button>
            <Button variant="outline" asChild>
              <Link to="/memory/settings">
                <Settings2 className="mr-1.5 h-4 w-4" /> Settings
              </Link>
            </Button>
          </div>
        }
      />
    );
  }
  return (
    <EmptyState
      icon={Moon}
      title="No runs yet"
      description="No dreaming runs have happened yet. Trigger one now, or wait for the next scheduled run."
      action={
        <Button onClick={onDream} disabled={triggering} className="bg-violet-600 text-white hover:bg-violet-500">
          <Sparkles className="mr-1.5 h-4 w-4" /> Dream now
        </Button>
      }
    />
  );
}

function Segmented({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-md px-3 py-1 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active
          ? "bg-accent font-medium text-accent-foreground"
          : "text-muted-foreground hover:bg-accent/50",
      )}
    >
      {children}
    </button>
  );
}

function FilterGroup({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center gap-1">
      <span className="text-2xs uppercase tracking-wide text-muted-foreground">{label}</span>
      {children}
    </div>
  );
}

function Chip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-full border px-2.5 py-0.5 text-2xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active
          ? "border-violet-500/40 bg-violet-500/10 text-violet-600 dark:text-violet-300"
          : "border-border bg-background text-muted-foreground hover:bg-accent/50",
      )}
    >
      {children}
    </button>
  );
}
