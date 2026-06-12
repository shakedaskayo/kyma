import { Brain, FolderSearch, Radar, type LucideIcon } from "lucide-react";
import { EmptyState } from "@/components/ui/empty-state";
import { Button } from "@/components/ui/button";
import { SkeletonRows } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { relTime } from "@/lib/time";
import type { DataSourceWatcher } from "@/sdk/datasources";
import { useDataSourceWatchers, useUpdateWatcherSettings, useWatcherSettings } from "./useDataSources";

const KINDS: Record<DataSourceWatcher["kind"], { label: string; icon: LucideIcon }> = {
  filedrop: { label: "File drop", icon: FolderSearch },
  cc_sync: { label: "Claude Code sync", icon: Brain },
};

/**
 * Live/stale pill — mirrors StatusBadge's emerald/amber palette, but watchers
 * carry a boolean `stale` rather than a DataSourceStatus, so the union doesn't
 * fit StatusBadge directly.
 */
export function LiveBadge({ stale, className }: { stale: boolean; className?: string }) {
  return (
    <div
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[11px] font-medium",
        stale
          ? "border-amber-300 bg-amber-50 text-amber-700 dark:border-amber-700/50 dark:bg-amber-950/40 dark:text-amber-400"
          : "border-emerald-300 bg-emerald-50 text-emerald-700 dark:border-emerald-700/50 dark:bg-emerald-950/40 dark:text-emerald-400",
        className,
      )}
    >
      <span
        className={cn(
          "h-1.5 w-1.5 rounded-full",
          stale ? "bg-amber-500" : "bg-emerald-500",
        )}
      />
      {stale ? "Stale" : "Live"}
    </div>
  );
}

/** "prefixes" (filedrop) or "root" (cc-sync) out of the untyped watcher config. */
export function watchedTarget(config: Record<string, unknown>): string {
  const prefixes = config.prefixes;
  if (Array.isArray(prefixes) && prefixes.length > 0) return prefixes.join(", ");
  if (typeof config.root === "string" && config.root) return config.root;
  return "—";
}

function pollInterval(config: Record<string, unknown>): string {
  const secs = config.poll_secs;
  return typeof secs === "number" && Number.isFinite(secs) ? `every ${secs}s` : "—";
}

export function formatScanDuration(ms: number): string {
  if (!Number.isFinite(ms)) return "—";
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}

function WatcherCard({ watcher }: { watcher: DataSourceWatcher }) {
  const kind = KINDS[watcher.kind] ?? { label: watcher.kind, icon: Radar };
  const Icon = kind.icon;
  const scan = watcher.last_scan;

  return (
    <div className="flex items-start gap-4 rounded-lg border bg-background px-4 py-3 shadow-sm">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border bg-background text-muted-foreground">
        <Icon size={18} />
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium text-foreground">{kind.label}</span>
          <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
            {watcher.node_host}
          </span>
          <span className="text-xs text-muted-foreground">as {watcher.identity}</span>
        </div>

        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
          <span className="min-w-0 truncate">
            Watching{" "}
            <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px] text-foreground">
              {watchedTarget(watcher.config)}
            </code>
          </span>
          <span>{pollInterval(watcher.config)}</span>
          <span>Heartbeat {relTime(watcher.last_heartbeat_at)}</span>
        </div>

        <div className="mt-0.5 text-xs text-muted-foreground">
          {scan ? (
            <span className={cn(scan.errors > 0 && "text-amber-600 dark:text-amber-400")}>
              Last scan: {scan.processed}/{scan.seen} files
              {scan.errors > 0 && `, ${scan.errors} ${scan.errors === 1 ? "error" : "errors"}`}
              {" · "}
              {formatScanDuration(scan.duration_ms)} · {relTime(scan.at)}
            </span>
          ) : (
            <span>No scans yet</span>
          )}
        </div>
      </div>

      <LiveBadge stale={watcher.stale} className="mt-0.5 shrink-0" />
    </div>
  );
}

export function WatchersTab() {
  const { data: watchers, isLoading, error } = useDataSourceWatchers();
  const { data: settings } = useWatcherSettings();
  const { mutate: updateSettings, isPending } = useUpdateWatcherSettings();

  if (isLoading) {
    return <SkeletonRows rows={4} className="mx-auto max-w-3xl py-2" />;
  }

  if (error) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-destructive">
        Failed to load watchers: {(error as Error).message}
      </div>
    );
  }

  if (!watchers || watchers.length === 0) {
    const ccEnabled = settings?.cc_sync_enabled ?? true;
    return (
      <EmptyState
        icon={Radar}
        title="No file watchers running"
        description="Watchers run alongside the engine and stream files or Claude Code memory into the context graph from the node where they live."
        action={
          <div className="flex flex-col items-center gap-3">
            {!ccEnabled && (
              <Button
                size="sm"
                onClick={() => updateSettings({ cc_sync_enabled: true })}
                disabled={isPending}
              >
                Enable Claude Code sync
              </Button>
            )}
            <p className="text-xs text-muted-foreground">
              File drop: set{" "}
              <code className="rounded bg-muted px-1 py-0.5 font-mono">KYMA_FILEDROP_ENABLED=1</code>
              {" "}and{" "}
              <code className="rounded bg-muted px-1 py-0.5 font-mono">KYMA_FILEDROP_PREFIXES</code>
              {" "}on the engine.
            </p>
          </div>
        }
      />
    );
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-2">
      {watchers.map((w) => (
        <WatcherCard key={w.id} watcher={w} />
      ))}
    </div>
  );
}
