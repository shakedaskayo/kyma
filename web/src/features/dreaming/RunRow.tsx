import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  Archive,
  Cloud,
  GitMerge,
  Link2,
  Plus,
  Timer,
} from "lucide-react";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { formatDuration, relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { Run } from "@/sdk/dreaming";
import { RunKindBadge } from "./RunKindBadge";
import { RunStatusBadge } from "./RunStatusBadge";
import { StatChip } from "./StatChip";

/**
 * A rich, clickable run row → drilldown. Running rows get a violet left accent +
 * faint violet wash + a live `current_phase` ticker; error rows get a rose
 * border and a truncated error with a tooltip.
 */
export function RunRow({ run }: { run: Run }) {
  const running = run.status === "running";
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);

  const s = run.stats;
  const engineModel = [run.engine, run.model].filter(Boolean).join(" · ");

  return (
    <Link
      to="/memory/dreaming/$runId"
      params={{ runId: run.id }}
      className={cn(
        "block rounded-xl border border-border/60 bg-card/40 px-3.5 py-2.5 shadow-elev-1 transition-[border-color,box-shadow,transform] hover:-translate-y-px hover:border-border-strong hover:bg-card/70 hover:shadow-elev-2",
        running && "bg-violet-500/[0.05]",
        run.status === "error" && "border-rose-500/30",
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

      <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-muted-foreground">
        <span className="font-medium capitalize text-foreground">{run.mode}</span>
        <span>·</span>
        <span className="capitalize">{run.trigger}</span>
        {engineModel && (
          <>
            <span>·</span>
            <span className="truncate font-mono text-[11px]" title={engineModel}>
              {engineModel}
            </span>
          </>
        )}
      </div>

      {running && run.progress?.current_phase && (
        <div className="mt-1.5 flex items-center gap-1.5 text-xs text-violet-500">
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-violet-500/60" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-violet-500" />
          </span>
          <span className="truncate">{run.progress.current_phase}</span>
        </div>
      )}

      {run.status === "error" && run.error && (
        <SimpleTooltip label={run.error}>
          <div className="mt-1.5 truncate text-xs text-rose-500">{run.error}</div>
        </SimpleTooltip>
      )}

      {s && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {s.memories_created > 0 && (
            <StatChip icon={Plus} label="created" value={s.memories_created} />
          )}
          {s.memories_merged > 0 && (
            <StatChip icon={GitMerge} label="merged" value={s.memories_merged} />
          )}
          {s.memories_archived > 0 && (
            <StatChip icon={Archive} label="archived" value={s.memories_archived} />
          )}
          {s.entities_linked > 0 && (
            <StatChip icon={Link2} label="linked" value={s.entities_linked} />
          )}
          {s.data_source_reads > 0 && (
            <StatChip icon={Cloud} label="reads" value={s.data_source_reads} />
          )}
        </div>
      )}
    </Link>
  );
}
