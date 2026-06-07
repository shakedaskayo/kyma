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
import type { Run, RunKind, RunStatus } from "@/sdk/dreaming";
import { cn } from "@/lib/utils";
import { relTime } from "@/lib/time";
import { useDreamingRuns, useTriggerDreaming } from "./useDreaming";
import { NodesStrip } from "./NodesStrip";
import { RunRow } from "./RunRow";
import { ActivityView } from "./ActivityView";

type SubTab = "runs" | "activity";
type KindFilter = "all" | RunKind;
type StatusFilter = "all" | RunStatus;

const INTERVAL_LABEL = (secs: number): string => {
  if (secs % 86400 === 0) return `${secs / 86400}d`;
  if (secs % 3600 === 0) return `${secs / 3600}h`;
  if (secs % 60 === 0) return `${secs / 60}m`;
  return `${secs}s`;
};

const PAGE = 20;

export function DreamingPanel() {
  const { endpoint, token } = useSession();
  const [subTab, setSubTab] = useState<SubTab>("runs");
  const [kindFilter, setKindFilter] = useState<KindFilter>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [limit, setLimit] = useState(PAGE);

  // Dreaming settings (enabled / mode / interval) drive the hero facts + empty state.
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
    () =>
      (runs ?? []).filter((r) => statusFilter === "all" || r.status === statusFilter),
    [runs, statusFilter],
  );

  const running = (runs ?? []).find((r) => r.status === "running") ?? null;
  const lastSettled = (runs ?? []).find((r) => r.status !== "running") ?? null;
  const hasMore = (runs?.length ?? 0) >= limit;

  return (
    <div className="flex h-full flex-col">
      {/* HERO */}
      <Hero
        dreaming={dreaming}
        running={running}
        lastSettled={lastSettled}
        onDream={() => trigger.mutate(undefined)}
        triggering={trigger.isPending}
        deduped={trigger.data?.job_id === null && !trigger.isPending}
      />

      {/* NODES */}
      <NodesStrip />

      {/* SUB-TABS */}
      <div className="flex items-center gap-1 border-b px-4 py-2">
        <Segmented active={subTab === "runs"} onClick={() => setSubTab("runs")}>
          Runs
        </Segmented>
        <Segmented active={subTab === "activity"} onClick={() => setSubTab("activity")}>
          Activity
        </Segmented>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {subTab === "activity" ? (
          <ActivityView />
        ) : (
          <div className="mx-auto max-w-3xl space-y-3 px-4 py-4">
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

            {/* List */}
            {isLoading && !runs ? (
              <SkeletonRows rows={5} />
            ) : error ? (
              <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                Failed to load runs: {(error as Error).message}
              </div>
            ) : filtered.length === 0 ? (
              <RunsEmptyState dreaming={dreaming} onDream={() => trigger.mutate(undefined)} triggering={trigger.isPending} />
            ) : (
              <>
                <div className="space-y-2">
                  {filtered.map((r) => (
                    <RunRow key={r.id} run={r} />
                  ))}
                </div>
                {hasMore && (
                  <div className="flex justify-center pt-1">
                    <Button variant="outline" size="sm" onClick={() => setLimit((l) => l + PAGE)}>
                      Load more
                    </Button>
                  </div>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function Hero({
  dreaming,
  running,
  lastSettled,
  onDream,
  triggering,
  deduped,
}: {
  dreaming: DreamingSettings | undefined;
  running: Run | null;
  lastSettled: Run | null;
  onDream: () => void;
  triggering: boolean;
  deduped: boolean;
}) {
  const isRunning = Boolean(running);
  const enabled = dreaming?.enabled ?? false;
  const subtitle = dreaming
    ? `${dreaming.mode.replace(/_/g, " ")} · every ${INTERVAL_LABEL(dreaming.interval_secs)}`
    : "Autonomous memory consolidation";

  return (
    <div
      className={cn(
        "relative overflow-hidden border-b bg-gradient-to-br from-violet-500/[0.07] via-background to-background px-6 py-5 transition-shadow",
        isRunning && "shadow-glow",
      )}
    >
      <div className="flex items-start gap-4">
        {/* Violet-halo Moon */}
        <div className="relative shrink-0">
          {isRunning && (
            <span className="absolute inset-0 -m-1 animate-ping rounded-2xl bg-violet-500/20" />
          )}
          <div className="relative rounded-2xl bg-violet-500/10 p-3 text-violet-500 ring-1 ring-inset ring-violet-500/20">
            <Moon className="h-7 w-7" />
          </div>
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h1 className="text-xl font-semibold tracking-tight">Dreaming</h1>
          </div>
          <p className="mt-0.5 text-sm capitalize text-muted-foreground">{subtitle}</p>

          {/* Status facts */}
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {isRunning ? (
              <StatusPill tone="running" pulse>
                Running
              </StatusPill>
            ) : enabled ? (
              <StatusPill tone="ok">Enabled</StatusPill>
            ) : (
              <StatusPill tone="idle">Off</StatusPill>
            )}
            {lastSettled && (
              <span className="text-2xs text-muted-foreground">
                Last run {relTime(lastSettled.finished_at ?? lastSettled.started_at)} ·{" "}
                <span className="capitalize">{lastSettled.status}</span>
                {lastSettled.stats?.summary ? ` · ${truncate(lastSettled.stats.summary, 80)}` : ""}
              </span>
            )}
          </div>
        </div>

        {/* Actions */}
        <div className="flex shrink-0 flex-col items-end gap-2">
          <Button onClick={onDream} disabled={isRunning || triggering}>
            <Sparkles className="mr-1.5 h-4 w-4" />
            {isRunning ? "Dreaming…" : "Dream now"}
          </Button>
          <Button variant="ghost" size="sm" asChild>
            <Link to="/memory/settings">
              <Settings2 className="mr-1.5 h-3.5 w-3.5" /> Settings
            </Link>
          </Button>
          {deduped && (
            <span className="text-2xs text-amber-500">A run is already in flight.</span>
          )}
        </div>
      </div>

      {/* Indeterminate shimmer progress while running */}
      {isRunning && (
        <div className="absolute inset-x-0 bottom-0 h-0.5 overflow-hidden bg-violet-500/10">
          <div className="h-full w-1/3 animate-shimmer bg-gradient-to-r from-transparent via-violet-500 to-transparent" />
        </div>
      )}
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
        title="Dreaming is off"
        description="Enable autonomous consolidation to let the agent distill, merge, and link memories on a schedule."
        action={
          <Button variant="outline" asChild>
            <Link to="/memory/settings">
              <Settings2 className="mr-1.5 h-4 w-4" /> Open settings
            </Link>
          </Button>
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
        <Button onClick={onDream} disabled={triggering}>
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
        "rounded-md px-3 py-1 text-sm transition-colors",
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
        "rounded-full border px-2.5 py-0.5 text-2xs transition-colors",
        active
          ? "border-violet-500/40 bg-violet-500/10 text-violet-600 dark:text-violet-300"
          : "border-border bg-background text-muted-foreground hover:bg-accent/50",
      )}
    >
      {children}
    </button>
  );
}

function truncate(s: string, n: number): string {
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
}
