import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  Archive,
  Cloud,
  GitMerge,
  Layers,
  Link2,
  RotateCw,
  Scale,
  ShieldCheck,
  Sparkles,
  Star,
  Timer,
  Wrench,
  X,
  Plus,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { SkeletonRows } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/ui/empty-state";
import { cn } from "@/lib/utils";
import { formatDuration } from "@/lib/time";
import { useNow } from "@/lib/use-now";
import type { LucideIcon } from "lucide-react";
import type { RunStats } from "@/sdk/dreaming";
import { useAgentRunTrace, useDreamingRun, useTriggerDreaming } from "./useDreaming";
import { RunKindBadge } from "./RunKindBadge";
import { RunStatusBadge } from "./RunStatusBadge";
import { ActivityFeed } from "./ActivityFeed";
import { ConversationView } from "./ConversationView";
import { traceToParts } from "./traceToParts";

type DetailTab = "activity" | "conversation";

export function RunDetail({ runId }: { runId: string }) {
  const { data: run, isLoading, error } = useDreamingRun(runId);
  const running = run?.status === "running";
  const now = useNow(running);
  const trigger = useTriggerDreaming();

  const { data: trace } = useAgentRunTrace(run?.agent_run_id ?? null, Boolean(running));
  const parts = useMemo(() => (trace ? traceToParts(trace.trace) : []), [trace]);

  // Default tab follows the run's status the first time it's observed — while
  // running, Activity is the more useful default; once finished, Conversation
  // (where the summary/reasoning lives) is. Doesn't yank the tab away from a
  // manual selection if status changes later while still viewing this run.
  const [tab, setTab] = useState<DetailTab>("activity");
  const defaultedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!run || defaultedFor.current === run.id) return;
    setTab(run.status === "running" ? "activity" : "conversation");
    defaultedFor.current = run.id;
  }, [run]);

  // Move focus into the pane header whenever the selected run changes, so
  // keyboard/screen-reader users land on the new content, not stranded on the
  // list row that was just activated. Keyed on `run` (not `runId`) because the
  // heading only exists once loading finishes — the ref is unset during the
  // skeleton phase — and guarded to fire once per run so it doesn't steal
  // focus back on every 2.5s poll refetch while a run is still in progress.
  const headingRef = useRef<HTMLHeadingElement>(null);
  const focusedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!run || focusedFor.current === run.id) return;
    headingRef.current?.focus();
    focusedFor.current = run.id;
  }, [run]);

  if (isLoading && !run) {
    return (
      <div className="px-4 py-6">
        <SkeletonRows rows={6} />
      </div>
    );
  }

  if (error || !run) {
    const notFound = (error as Error | undefined)?.message === "not found";
    return (
      <EmptyState
        icon={Sparkles}
        title={notFound ? "Run not found" : "Failed to load run"}
        description={(error as Error | undefined)?.message}
        action={
          <Button variant="outline" asChild>
            <Link to="/memory/dreaming">Clear selection</Link>
          </Button>
        }
      />
    );
  }

  const engineModel = [run.engine, run.model].filter(Boolean).join(" · ");

  return (
    <div className="flex h-full flex-col">
      {/* Pane header */}
      <div className="flex items-center gap-2 border-b border-border/60 bg-background/85 px-4 py-3 backdrop-blur-md">
        <RunKindBadge kind={run.kind} />
        <RunStatusBadge status={run.status} />
        <h1
          ref={headingRef}
          tabIndex={-1}
          className="ml-1 truncate text-base font-semibold capitalize focus:outline-none"
        >
          {run.mode} run
        </h1>
        <div className="ml-auto flex items-center gap-3 text-2xs text-muted-foreground">
          <span className="hidden items-center gap-1 tabular-nums sm:flex">
            <Timer className="h-3.5 w-3.5" />
            {formatDuration(run.started_at, run.finished_at, now)}
          </span>
          {engineModel && <span className="hidden font-mono md:inline">{engineModel}</span>}
          <span className="hidden capitalize lg:inline">{run.trigger}</span>
          {run.worker_id && (
            <span className="hidden font-mono lg:inline" title={run.worker_id}>
              node {run.worker_id.slice(0, 8)}
            </span>
          )}
          <Link
            to="/memory/review"
            search={{ source_run_id: runId }}
            className="flex items-center gap-1 underline hover:text-foreground"
            title="Review this run's gated memory candidates"
          >
            <ShieldCheck className="h-3.5 w-3.5" /> Candidates
          </Link>
          <Button variant="ghost" size="icon" asChild className="h-7 w-7">
            <Link to="/memory/dreaming" aria-label="Close run detail">
              <X className="h-4 w-4" />
            </Link>
          </Button>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <div className="space-y-5 px-4 py-4">
          {/* Currently (running) */}
          {running && run.progress && (
            <div className="rounded-lg border-l-2 border-l-violet-500 bg-violet-500/[0.05] p-3">
              <div className="flex items-center gap-1.5 text-2xs font-medium uppercase tracking-wide text-violet-500">
                <span className="relative flex h-1.5 w-1.5">
                  <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-violet-500/60" />
                  <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-violet-500" />
                </span>
                Currently
              </div>
              <p className="mt-1 text-sm text-foreground">{run.progress.current_phase}</p>
              {run.progress.thinking && (
                <p className="mt-1 text-xs italic text-muted-foreground">{run.progress.thinking}</p>
              )}
            </div>
          )}

          {/* Error */}
          {run.error && (
            <div className="rounded-lg border border-destructive bg-destructive/5 p-3 text-sm text-destructive">
              <div className="flex items-center justify-between gap-2">
                <div className="text-2xs font-medium uppercase tracking-wide">Error</div>
                <Button
                  size="xs"
                  variant="outline"
                  onClick={() => trigger.mutate({ mode: run.mode })}
                  disabled={trigger.isPending}
                >
                  <RotateCw className="mr-1 h-3 w-3" /> Retry
                </Button>
              </div>
              <p className="mt-1 break-words">{run.error}</p>
            </div>
          )}

          {/* Stats */}
          {run.stats && (
            <Section title="Outcome">
              <StatGrid stats={run.stats} />
              {run.stats.summary && (
                <div className="mt-3 rounded-lg border-l-2 border-l-violet-400/70 bg-violet-500/[0.05] py-2.5 pl-3.5 pr-3">
                  <div className="mb-1 flex items-center gap-1.5 text-2xs font-medium uppercase tracking-[0.14em] text-violet-300">
                    <Sparkles className="h-3.5 w-3.5" /> Summary
                  </div>
                  <p className="text-sm leading-relaxed text-foreground/85">{run.stats.summary}</p>
                </div>
              )}
            </Section>
          )}

          {/* Activity / Conversation */}
          <section>
            <div className="mb-2 flex items-center gap-1 border-b border-border/60">
              <TabButton active={tab === "activity"} onClick={() => setTab("activity")}>
                Activity
              </TabButton>
              <TabButton active={tab === "conversation"} onClick={() => setTab("conversation")}>
                Conversation
              </TabButton>
            </div>
            {tab === "activity" ? (
              run.progress && run.progress.activity.length > 0 ? (
                <ActivityFeed items={run.progress.activity} live={running} />
              ) : (
                <p className="px-1 py-4 text-xs text-muted-foreground">No activity recorded.</p>
              )
            ) : run.agent_run_id ? (
              <ConversationView question={trace?.question ?? ""} parts={parts} live={running} />
            ) : (
              <EmptyState
                icon={Sparkles}
                title="No conversation recorded"
                description="This run did not record an agent conversation trace."
                className="py-10"
              />
            )}
          </section>
        </div>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section>
      <div className="mb-2 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}
      </div>
      {children}
    </section>
  );
}

function TabButton({
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
        "relative px-2.5 pb-2 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        active ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
      {active && (
        <span className="absolute inset-x-1 -bottom-px h-0.5 rounded-full bg-violet-400" />
      )}
    </button>
  );
}

const STAT_CARDS: { key: keyof RunStats; label: string; icon: LucideIcon; accent?: boolean }[] = [
  { key: "memories_created", label: "Created", icon: Plus },
  { key: "memories_merged", label: "Merged", icon: GitMerge },
  { key: "memories_archived", label: "Archived", icon: Archive },
  { key: "importance_rescored", label: "Rescored", icon: Star },
  { key: "judgements", label: "Judgements", icon: Scale },
  { key: "entities_linked", label: "Linked", icon: Link2 },
  { key: "data_source_reads", label: "Reads", icon: Cloud },
  { key: "tool_calls", label: "Tool calls", icon: Wrench },
  { key: "schemas_induced", label: "Schemas induced", icon: Layers, accent: true },
];

function StatGrid({ stats }: { stats: RunStats }) {
  return (
    <div className="grid grid-cols-2 gap-2">
      {STAT_CARDS.map(({ key, label, icon: Icon, accent }) => {
        const value = (stats[key] as number) ?? 0;
        const active = value > 0;
        return (
          <div
            key={key}
            className={cn(
              "rounded-xl border border-border/60 bg-card/40 p-3 shadow-elev-1",
              !active && "opacity-50",
            )}
          >
            <div
              className={cn(
                "flex items-center gap-1.5 text-2xs",
                active && accent ? "text-cyan-500" : "text-muted-foreground",
              )}
            >
              <Icon className="h-3.5 w-3.5" /> {label}
            </div>
            <div
              className={cn(
                "mt-1 text-lg font-semibold tabular-nums",
                active && accent && "text-cyan-500",
              )}
            >
              {value}
            </div>
          </div>
        );
      })}
    </div>
  );
}
