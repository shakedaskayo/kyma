import { useEffect, useMemo, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  Archive,
  ArrowLeft,
  Cloud,
  GitMerge,
  Link2,
  Plus,
  Scale,
  ShieldCheck,
  Sparkles,
  Star,
  Timer,
  Wrench,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { SkeletonRows } from "@/components/ui/skeleton";
import { EmptyState } from "@/components/ui/empty-state";
import { cn } from "@/lib/utils";
import { formatDuration } from "@/lib/time";
import type { LucideIcon } from "lucide-react";
import type { RunStats } from "@/sdk/dreaming";
import { useAgentRunTrace, useDreamingRun } from "./useDreaming";
import { RunKindBadge } from "./RunKindBadge";
import { RunStatusBadge } from "./RunStatusBadge";
import { ActivityFeed } from "./ActivityFeed";
import { ConversationView } from "./ConversationView";
import { traceToParts } from "./traceToParts";

export function RunDetail({ runId }: { runId: string }) {
  const { data: run, isLoading, error } = useDreamingRun(runId);
  const running = run?.status === "running";

  // Live-ticking clock for the header duration when running.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);

  const { data: trace } = useAgentRunTrace(run?.agent_run_id ?? null, Boolean(running));
  const parts = useMemo(() => (trace ? traceToParts(trace.trace) : []), [trace]);

  if (isLoading && !run) {
    return (
      <div className="mx-auto max-w-3xl px-4 py-6">
        <SkeletonRows rows={6} />
      </div>
    );
  }

  if (error || !run) {
    return (
      <EmptyState
        icon={Sparkles}
        title={(error as Error | undefined)?.message === "not found" ? "Run not found" : "Failed to load run"}
        description={(error as Error | undefined)?.message}
        action={
          <Button variant="outline" asChild>
            <Link to="/memory/dreaming">Back to dreaming</Link>
          </Button>
        }
      />
    );
  }

  const engineModel = [run.engine, run.model].filter(Boolean).join(" · ");

  return (
    <div className="flex h-full flex-col">
      {/* Sticky run header */}
      <div className="sticky top-0 z-10 border-b border-border/60 bg-background/85 px-4 py-3 backdrop-blur-md">
        <div className="mx-auto flex max-w-3xl items-center gap-2">
          <Button variant="ghost" size="icon" asChild className="h-8 w-8">
            <Link to="/memory/dreaming">
              <ArrowLeft className="h-4 w-4" />
            </Link>
          </Button>
          <RunKindBadge kind={run.kind} />
          <RunStatusBadge status={run.status} />
          <h1 className="ml-1 text-base font-semibold capitalize">{run.mode} run</h1>
          <div className="ml-auto flex items-center gap-3 text-2xs text-muted-foreground">
            <span className="flex items-center gap-1 tabular-nums">
              <Timer className="h-3.5 w-3.5" />
              {formatDuration(run.started_at, run.finished_at, now)}
            </span>
            {engineModel && <span className="font-mono">{engineModel}</span>}
            <span className="capitalize">{run.trigger}</span>
            {run.worker_id && (
              <span className="font-mono" title={run.worker_id}>
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
          </div>
        </div>
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        <div className="mx-auto max-w-3xl space-y-6 px-4 py-5">
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
              <div className="text-2xs font-medium uppercase tracking-wide">Error</div>
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

          {/* Activity */}
          {run.progress && run.progress.activity.length > 0 && (
            <Section title="Activity">
              <ActivityFeed items={run.progress.activity} live={running} />
            </Section>
          )}

          {/* Conversation */}
          <Section title="Conversation">
            {run.agent_run_id ? (
              <ConversationView
                question={trace?.question ?? ""}
                parts={parts}
                live={running}
              />
            ) : (
              <EmptyState
                icon={Sparkles}
                title="No conversation recorded"
                description="This run did not record an agent conversation trace."
                className="py-10"
              />
            )}
          </Section>
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

const STAT_CARDS: { key: keyof RunStats; label: string; icon: LucideIcon }[] = [
  { key: "memories_created", label: "Created", icon: Plus },
  { key: "memories_merged", label: "Merged", icon: GitMerge },
  { key: "memories_archived", label: "Archived", icon: Archive },
  { key: "importance_rescored", label: "Rescored", icon: Star },
  { key: "judgements", label: "Judgements", icon: Scale },
  { key: "entities_linked", label: "Linked", icon: Link2 },
  { key: "data_source_reads", label: "Reads", icon: Cloud },
  { key: "tool_calls", label: "Tool calls", icon: Wrench },
];

function StatGrid({ stats }: { stats: RunStats }) {
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
      {STAT_CARDS.map(({ key, label, icon: Icon }) => {
        const value = stats[key] as number;
        return (
          <div
            key={key}
            className={cn(
              "rounded-xl border border-border/60 bg-card/40 p-3 shadow-elev-1",
              value > 0 ? "" : "opacity-50",
            )}
          >
            <div className="flex items-center gap-1.5 text-2xs text-muted-foreground">
              <Icon className="h-3.5 w-3.5" /> {label}
            </div>
            <div className="mt-1 text-lg font-semibold tabular-nums">{value}</div>
          </div>
        );
      })}
    </div>
  );
}
