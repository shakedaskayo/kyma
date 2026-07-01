import { Link } from "@tanstack/react-router";
import { Archive, Cloud, GitMerge, Link2, Plus, Timer } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { formatDuration, relTime } from "@/lib/time";
import { useNow } from "@/lib/use-now";
import { cn } from "@/lib/utils";
import type { Run, RunStats } from "@/sdk/dreaming";
import { RunKindBadge } from "./RunKindBadge";
import { RunStatusBadge } from "./RunStatusBadge";
import { StatChip } from "./StatChip";

const STAT_PRIORITY: { key: keyof RunStats; icon: LucideIcon; label: string }[] = [
  { key: "memories_created", icon: Plus, label: "created" },
  { key: "memories_merged", icon: GitMerge, label: "merged" },
  { key: "memories_archived", icon: Archive, label: "archived" },
  { key: "entities_linked", icon: Link2, label: "linked" },
  { key: "data_source_reads", icon: Cloud, label: "reads" },
];

const MAX_VISIBLE_CHIPS = 3;

/**
 * A rich, clickable run row → drilldown. Running rows get a violet left accent
 * + faint violet wash + a live `current_phase` ticker; error rows get a rose
 * border and a truncated error with a tooltip.
 */
export function RunRow({ run, active }: { run: Run; active?: boolean }) {
  const running = run.status === "running";
  const now = useNow(running);

  const s = run.stats;
  const engineModel = [run.engine, run.model].filter(Boolean).join(" · ");

  const entries = s ? STAT_PRIORITY.filter((e) => (s[e.key] as number) > 0) : [];
  const shown = entries.slice(0, MAX_VISIBLE_CHIPS);
  const overflow = entries.slice(MAX_VISIBLE_CHIPS);

  return (
    <Link
      to="/memory/dreaming/$runId"
      params={{ runId: run.id }}
      replace
      className={cn(
        "block rounded-xl border border-border/60 bg-card/40 px-3.5 py-2.5 shadow-elev-1 transition-[border-color,box-shadow,transform] hover:-translate-y-px hover:border-border-strong hover:bg-card/70 hover:shadow-elev-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
        running && "bg-violet-500/[0.05]",
        run.status === "error" && "border-rose-500/30",
        active && "border-violet-400/50 bg-violet-500/[0.06] ring-1 ring-violet-400/30",
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        <RunKindBadge kind={run.kind} />
        <RunStatusBadge status={run.status} />
        <span className="ml-auto flex items-center gap-2 text-2xs text-muted-foreground">
          <span title={run.started_at}>{relTime(run.started_at, now)}</span>
          <span className="flex items-center gap-1 tabular-nums">
            <Timer className="h-3 w-3" />
            {formatDuration(run.started_at, run.finished_at, now)}
          </span>
        </span>
      </div>

      <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
        <span className="font-medium capitalize text-foreground">{run.mode}</span>
        <span>·</span>
        <span className="capitalize">{run.trigger}</span>
        {engineModel && (
          <>
            <span>·</span>
            <span className="min-w-0 truncate font-mono text-[11px]" title={engineModel}>
              {engineModel}
            </span>
          </>
        )}
      </div>

      {running && run.progress?.current_phase && (
        <div className="mt-1.5 flex items-center gap-1.5 text-xs text-violet-500">
          <span className="relative flex h-1.5 w-1.5 shrink-0">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-violet-500/60" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-violet-500" />
          </span>
          <span className="truncate">{run.progress.current_phase}</span>
        </div>
      )}

      {run.status === "error" && run.error && (
        <SimpleTooltip label={run.error}>
          <p className="mt-1.5 truncate text-xs text-rose-500">{run.error}</p>
        </SimpleTooltip>
      )}

      {shown.length > 0 && (
        <div className="mt-2 flex flex-wrap items-center gap-1.5">
          {shown.map(({ key, icon, label }) => (
            <StatChip key={key} icon={icon} label={label} value={s![key] as number} />
          ))}
          {overflow.length > 0 && (
            <SimpleTooltip
              label={overflow.map(({ key, label }) => `${s![key]} ${label}`).join(", ")}
            >
              <span className="inline-flex items-center rounded-md border bg-muted/40 px-2 py-0.5 text-2xs text-muted-foreground">
                +{overflow.length}
              </span>
            </SimpleTooltip>
          )}
        </div>
      )}
    </Link>
  );
}
