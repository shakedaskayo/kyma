import { Link } from "@tanstack/react-router";
import { useQuery } from "@tanstack/react-query";
import { Settings2, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { StatusPill } from "@/components/ui/status";
import { useSession } from "@/sdk/session";
import { getMemorySettings, type DreamingSettings } from "@/sdk/memory";
import { cn } from "@/lib/utils";
import { intervalLabel } from "@/features/memory/lib";
import { useDreamingRuns, useTriggerDreaming } from "./useDreaming";

/**
 * Full-width action strip above the two-pane split: engine status, the live
 * phase ticker for whatever's currently running, "Dream now", and a Settings
 * link. `MemoryHeader` above already carries the page title/nav, so this
 * strip renders no heading of its own.
 */
export function DreamingToolbar() {
  const { endpoint, token } = useSession();

  const { data: settings } = useQuery({
    queryKey: ["memory-settings", endpoint],
    queryFn: () => getMemorySettings({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    staleTime: 30_000,
  });
  const dreaming: DreamingSettings | undefined = settings?.dreaming;

  const { data: runs } = useDreamingRuns({ limit: 5 });
  const trigger = useTriggerDreaming();

  const running = (runs ?? []).find((r) => r.status === "running") ?? null;
  const isRunning = Boolean(running);

  return (
    <div className="border-b border-border/60 bg-surface/40">
      <div className="flex flex-wrap items-center gap-3 px-6 py-3">
        <div className="flex items-center gap-2">
          {isRunning ? (
            <StatusPill tone="running" pulse>
              Dreaming now
            </StatusPill>
          ) : dreaming?.enabled ? (
            <StatusPill tone="ok">Armed · every {intervalLabel(dreaming.interval_secs)}</StatusPill>
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
  );
}
