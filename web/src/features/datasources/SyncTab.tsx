import { Link } from "@tanstack/react-router";
import { Brain } from "lucide-react";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { relTime } from "@/lib/time";
import type { DataSourceWatcher } from "@/sdk/datasources";
import { useDataSourceWatchers } from "./useDataSources";
import { LiveBadge, watchedTarget } from "./WatchersTab";

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

const COLUMNS: { key: keyof Omit<RealmSyncRow, "realm">; label: string }[] = [
  { key: "upserted", label: "Upserted" },
  { key: "skipped", label: "Skipped" },
  { key: "user_edited", label: "User edited" },
  { key: "edges_added", label: "Edges" },
  { key: "archived", label: "Archived" },
];

function SyncCard({ watcher }: { watcher: DataSourceWatcher }) {
  const realms = realmRows(watcher);

  return (
    <div className="rounded-lg border bg-background px-4 py-3 shadow-sm">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="text-foreground">
          Watching{" "}
          <code className="rounded bg-muted px-1 py-0.5 font-mono text-[12px]">
            {watchedTarget(watcher.config)}
          </code>{" "}
          on <span className="font-medium">{watcher.node_host}</span> as{" "}
          <span className="font-medium">{watcher.identity}</span>
        </span>
        <span className="text-xs text-muted-foreground">
          · last sync {relTime(watcher.last_scan?.at ?? watcher.last_heartbeat_at)}
        </span>
        <LiveBadge stale={watcher.stale} className="ml-auto" />
      </div>

      {realms.length === 0 ? (
        <p className="mt-3 text-sm text-muted-foreground">No realms synced yet.</p>
      ) : (
        <table className="mt-3 w-full border-collapse text-sm">
          <thead>
            <tr className="text-[10px] uppercase tracking-wider text-muted-foreground">
              <th className="pb-1 pr-4 text-left font-semibold">Realm</th>
              {COLUMNS.map((c) => (
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
                {COLUMNS.map((c) => (
                  <td
                    key={c.key}
                    className="py-1.5 pr-4 text-right tabular-nums text-muted-foreground"
                  >
                    {r[c.key]}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}

export function SyncTab() {
  const { data: watchers, isLoading, error } = useDataSourceWatchers();

  if (isLoading) {
    return <SkeletonRows rows={4} className="mx-auto max-w-3xl py-2" />;
  }

  if (error) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-destructive">
        Failed to load sync status: {(error as Error).message}
      </div>
    );
  }

  const ccSync = (watchers ?? []).filter((w) => w.kind === "cc_sync");

  if (ccSync.length === 0) {
    return (
      <EmptyState
        icon={Brain}
        title="Claude Code sync is not running"
        description="When enabled, the engine watches your local Claude Code memory directories and mirrors them into the memory store, one realm per project."
        action={
          <p className="text-xs text-muted-foreground">
            Set <code className="rounded bg-muted px-1 py-0.5 font-mono">KYMA_CC_WATCH=1</code>{" "}
            on the node running kyma to enable it.
          </p>
        }
      />
    );
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-3">
      {ccSync.map((w) => (
        <SyncCard key={w.id} watcher={w} />
      ))}
      <p className="text-xs text-muted-foreground">
        Each realm maps to a project's memories — browse and search them in{" "}
        <Link to="/memory" className="text-primary underline-offset-2 hover:underline">
          Memory
        </Link>
        .
      </p>
    </div>
  );
}
