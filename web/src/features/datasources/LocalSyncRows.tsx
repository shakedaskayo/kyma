import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronDown, FolderSearch } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { relTime } from "@/lib/time";
import type { DataSourceWatcher } from "@/sdk/datasources";
import { BrandIcon } from "./BrandIcon";
import {
  useDataSourceWatchers,
  useUpdateWatcherSettings,
  useWatcherSettings,
} from "./useDataSources";

// Rows for node-local sync (watchers): the pre-installed Claude Code memory
// sync and any filedrop watchers, rendered in the same unified sources list
// as configured data sources. Absorbs the old Watchers + Memory Sync tabs.

/**
 * Live/stale pill — mirrors StatusBadge's emerald/amber palette, but watchers
 * carry a boolean `stale` rather than a DataSourceStatus.
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
      <span className={cn("h-1.5 w-1.5 rounded-full", stale ? "bg-amber-500" : "bg-emerald-500")} />
      {stale ? "Stale" : "Live"}
    </div>
  );
}

/** "prefixes" (filedrop) or "root" (cc-sync/obsidian) out of the watcher config. */
export function watchedTarget(config: Record<string, unknown>): string {
  const prefixes = config.prefixes;
  if (Array.isArray(prefixes) && prefixes.length > 0) return prefixes.join(", ");
  if (typeof config.root === "string" && config.root) return config.root;
  return "—";
}

export function formatScanDuration(ms: number): string {
  if (!Number.isFinite(ms)) return "—";
  return ms < 1000 ? `${ms}ms` : `${(ms / 1000).toFixed(1)}s`;
}

function pollInterval(config: Record<string, unknown>): string {
  const secs = config.poll_secs;
  return typeof secs === "number" && Number.isFinite(secs) ? `every ${secs}s` : "—";
}

/** Per-realm counters from a cc-sync scan (`last_scan.detail.realms`). */
interface RealmSyncRow {
  realm: string;
  upserted: number;
  skipped: number;
  user_edited: number;
  edges_added: number;
  archived: number;
}

function realmRows(watcher: DataSourceWatcher): RealmSyncRow[] {
  const realms = watcher.last_scan?.detail?.realms;
  return Array.isArray(realms) ? (realms as RealmSyncRow[]) : [];
}

const REALM_COLUMNS: { key: keyof Omit<RealmSyncRow, "realm">; label: string }[] = [
  { key: "upserted", label: "Upserted" },
  { key: "skipped", label: "Skipped" },
  { key: "user_edited", label: "User edited" },
  { key: "edges_added", label: "Edges" },
  { key: "archived", label: "Archived" },
];

function RealmTable({ realms }: { realms: RealmSyncRow[] }) {
  if (realms.length === 0) {
    return <p className="mt-3 text-sm text-muted-foreground">No realms synced yet.</p>;
  }
  return (
    <table className="mt-3 w-full border-collapse text-sm">
      <thead>
        <tr className="text-[10px] uppercase tracking-wider text-muted-foreground">
          <th className="pb-1 pr-4 text-left font-semibold">Realm</th>
          {REALM_COLUMNS.map((c) => (
            <th key={c.key} className="pb-1 pr-4 text-right font-semibold">
              {c.label}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {realms.map((r) => (
          <tr key={r.realm} className="border-t border-muted/40">
            <td className="py-1.5 pr-4 align-top">
              <code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-foreground">
                {r.realm}
              </code>
            </td>
            {REALM_COLUMNS.map((c) => (
              <td key={c.key} className="py-1.5 pr-4 text-right tabular-nums text-muted-foreground">
                {r[c.key]}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/**
 * The pre-installed Claude Code memory-sync source. Always rendered at the
 * top of the unified sources list — live with its watcher state when the
 * sync loop is running, otherwise as a paused/starting placeholder with the
 * enable toggle.
 */
export function CcSyncRow() {
  const { data: watchers } = useDataSourceWatchers();
  const { data: settings } = useWatcherSettings();
  const { mutate: updateSettings, isPending } = useUpdateWatcherSettings();
  const [expanded, setExpanded] = useState(false);

  const ccWatchers = (watchers ?? []).filter((w) => w.kind === "cc_sync");
  const enabled = settings?.cc_sync_enabled ?? true;
  const primary = ccWatchers[0];
  const realms = ccWatchers.flatMap(realmRows);

  return (
    <div className="rounded-lg border bg-background px-4 py-3 shadow-sm">
      <div className="flex items-center gap-4">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border bg-background">
          <BrandIcon brand="claude" size={18} />
        </div>

        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="font-medium text-foreground">Claude Code</span>
            <span className="rounded bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-primary">
              Pre-installed
            </span>
            <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
              Memory sync
            </span>
            {primary && (
              <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
                {primary.node_host}
              </span>
            )}
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
            {primary ? (
              <>
                <span className="min-w-0 truncate">
                  Watching{" "}
                  <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px] text-foreground">
                    {watchedTarget(primary.config)}
                  </code>
                </span>
                <span>{pollInterval(primary.config)}</span>
                <span>
                  Last sync {relTime(primary.last_scan?.at ?? primary.last_heartbeat_at)}
                </span>
              </>
            ) : enabled ? (
              <span>Starting — the first scan will appear shortly.</span>
            ) : (
              <span>Paused — Claude Code memory is not being synced.</span>
            )}
          </div>
        </div>

        {primary && <LiveBadge stale={primary.stale} className="shrink-0" />}

        <div className="flex shrink-0 items-center gap-1">
          {enabled ? (
            <Button
              variant="ghost"
              size="sm"
              className="text-xs text-muted-foreground"
              onClick={() => updateSettings({ cc_sync_enabled: false })}
              disabled={isPending}
            >
              Pause
            </Button>
          ) : (
            <Button
              size="sm"
              onClick={() => updateSettings({ cc_sync_enabled: true })}
              disabled={isPending}
            >
              Enable
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            aria-label="details"
            title="Details"
            onClick={() => setExpanded((e) => !e)}
          >
            <ChevronDown className={cn("transition-transform", expanded && "rotate-180")} />
          </Button>
        </div>
      </div>

      {expanded && (
        <div className="mt-2 border-t border-muted/40 pt-1">
          <RealmTable realms={realms} />
          <p className="mt-3 text-xs text-muted-foreground">
            Each realm maps to a project's memories — browse and search them in{" "}
            <Link to="/memory" className="text-primary underline-offset-2 hover:underline">
              Memory →
            </Link>
          </p>
        </div>
      )}
    </div>
  );
}

/** A filedrop watcher, rendered in the unified sources list. */
export function FiledropRow({ watcher }: { watcher: DataSourceWatcher }) {
  const scan = watcher.last_scan;
  return (
    <div className="flex items-center gap-4 rounded-lg border bg-background px-4 py-3 shadow-sm">
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border bg-background text-muted-foreground">
        <FolderSearch size={18} />
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="font-medium text-foreground">File drop</span>
          <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
            {watcher.node_host}
          </span>
          <span className="text-xs text-muted-foreground">as {watcher.identity}</span>
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
          <span className="min-w-0 truncate">
            Watching{" "}
            <code className="rounded bg-muted px-1 py-0.5 font-mono text-[11px] text-foreground">
              {watchedTarget(watcher.config)}
            </code>
          </span>
          <span>{pollInterval(watcher.config)}</span>
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

      <LiveBadge stale={watcher.stale} className="shrink-0" />
    </div>
  );
}
