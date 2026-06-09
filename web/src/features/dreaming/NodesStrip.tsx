import { useState } from "react";
import { ChevronDown, Cpu, Server } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { StatusDot, type StatusTone } from "@/components/ui/status";
import { SimpleTooltip } from "@/components/ui/tooltip";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import type { Worker, WorkerStatus } from "@/sdk/dreaming";
import { workerAgents } from "@/sdk/dreaming";
import { useWorkers } from "./useDreaming";

const STATUS_TONE: Record<WorkerStatus, StatusTone> = {
  online: "ok",
  draining: "warn",
  offline: "idle",
};

/**
 * Collapsible "Nodes (n)" strip — a horizontal scroll of compact worker cards.
 * Online workers with active presence pulse; capabilities show as muted chips
 * (max 3 + overflow); presence + heartbeat read in the footer.
 */
export function NodesStrip() {
  const { data: workers, isLoading } = useWorkers();
  const [open, setOpen] = useState(true);

  const count = workers?.length ?? 0;

  if (!isLoading && count === 0) return null;

  return (
    <div className="border-b bg-background/40">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        className="flex w-full items-center gap-2 px-4 py-2 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground"
      >
        <Server className="h-3.5 w-3.5" />
        Nodes ({count})
        <ChevronDown
          className={cn("h-3.5 w-3.5 transition-transform", open ? "rotate-180" : "")}
        />
      </button>
      {open && (
        <div className="flex gap-2 overflow-x-auto px-4 pb-3">
          {isLoading && count === 0
            ? Array.from({ length: 2 }).map((_, i) => (
                <div key={i} className="h-28 w-56 shrink-0 animate-pulse rounded-lg border bg-muted/40" />
              ))
            : workers!.map((w) => <NodeCard key={w.id} worker={w} />)}
        </div>
      )}
    </div>
  );
}

function NodeCard({ worker }: { worker: Worker }) {
  const activeSessions = worker.presence?.length ?? 0;
  const tone = STATUS_TONE[worker.status];
  const pulse = worker.status === "online" && activeSessions > 0;
  const caps = worker.capabilities ?? [];
  const shown = caps.slice(0, 3);
  const overflow = caps.length - shown.length;
  // Detected coding agents this node owns as local sources (new `agents`
  // array, with a fallback to the legacy claude_code compat key).
  const agents = workerAgents(worker.sources);

  return (
    <div className="flex w-56 shrink-0 flex-col gap-1.5 rounded-lg border bg-card p-3 shadow-elev-1">
      <div className="flex items-center gap-1.5">
        <StatusDot tone={tone} pulse={pulse} title={worker.status} />
        <span className="truncate font-mono text-xs text-foreground" title={worker.hostname}>
          {worker.hostname || worker.name || worker.id.slice(0, 8)}
        </span>
        <Badge
          tone={worker.kind === "daemon" ? "indigo" : "slate"}
          className="ml-auto shrink-0 px-1.5 py-0 text-[10px]"
        >
          <Cpu className="h-2.5 w-2.5" />
          {worker.kind}
        </Badge>
      </div>

      {shown.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {shown.map((c) => (
            <span
              key={c}
              className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground"
            >
              {c}
            </span>
          ))}
          {overflow > 0 && (
            <SimpleTooltip label={caps.slice(3).join(", ")}>
              <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                +{overflow}
              </span>
            </SimpleTooltip>
          )}
        </div>
      )}

      {agents.length > 0 && (
        <SimpleTooltip
          label={agents
            .map((a) => `${a.name}${a.realms.length ? ` · ${a.realms.length} realms` : ""}`)
            .join(", ")}
        >
          <div className="truncate text-[10px] text-muted-foreground">
            sources: {agents.map((a) => a.name).join(", ")}
          </div>
        </SimpleTooltip>
      )}

      <div className="mt-auto flex items-center justify-between text-[10px] text-muted-foreground">
        {activeSessions > 0 ? (
          <span className="text-violet-500">
            ● {activeSessions} active session{activeSessions === 1 ? "" : "s"}
          </span>
        ) : (
          <span>no active sessions</span>
        )}
        <span className="tabular-nums">
          {worker.last_heartbeat ? `♥ ${relTime(worker.last_heartbeat)}` : "no heartbeat"}
        </span>
      </div>
    </div>
  );
}
