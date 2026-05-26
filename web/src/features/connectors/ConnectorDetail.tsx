import { useNavigate } from "@tanstack/react-router";
import {
  AlertTriangle,
  Database,
  Network,
  Pause,
  Play,
  RefreshCw,
  Table as TableIcon,
  Trash2,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useGraphStore } from "@/features/graph/graph-store";
import { deriveStatus, type ConnectorDetail as Detail } from "@/sdk/connectors";
import { formatInterval, graphNameForType, kindByType } from "./connector-kinds";
import { StatusBadge } from "./StatusBadge";
import {
  useDeleteConnector,
  usePauseConnector,
  useResumeConnector,
  useTriggerConnector,
} from "./useConnectors";

function fmtDate(iso: string | null): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "—";
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function ConnectorDetail({
  detail,
  onDeleted,
}: {
  detail: Detail;
  onDeleted: () => void;
}) {
  const navigate = useNavigate();
  const trigger = useTriggerConnector(detail.id);
  const pause = usePauseConnector(detail.id);
  const resume = useResumeConnector(detail.id);
  const del = useDeleteConnector();

  const kind = kindByType(detail.type);
  const Icon = kind?.icon;
  const status = deriveStatus(detail);

  const repos = (detail.config?.repos as string[] | undefined) ?? [];

  const openGraph = () => {
    // GraphView reads the active graph from the store — set it, then navigate.
    const name = graphNameForType(detail.type);
    useGraphStore.getState().setGraph(name);
    void navigate({ to: "/graph" });
  };

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      {/* Header */}
      <div className="flex items-start gap-4">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary/80">
          {Icon ? <Icon className="h-5 w-5" /> : null}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-lg font-semibold">{detail.name}</h1>
            <StatusBadge status={status} />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
            <span className="capitalize">{kind?.label ?? detail.type}</span>
            <span className="inline-flex items-center gap-1">
              <Database className="h-3 w-3" /> {detail.target_database}
            </span>
            <span className="inline-flex items-center gap-1">
              <TableIcon className="h-3 w-3" /> {detail.target_table}
            </span>
            <span className="inline-flex items-center gap-1">
              <RefreshCw className="h-3 w-3" /> {formatInterval(detail.schedule_ms)}
            </span>
          </div>
        </div>
        <Button variant="outline" size="sm" onClick={openGraph}>
          <Network className="mr-1.5 h-3.5 w-3.5" /> Open graph
        </Button>
      </div>

      {/* Toolbar */}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          onClick={() => trigger.mutate()}
          disabled={trigger.isPending || !detail.enabled}
        >
          <RefreshCw className={`mr-1.5 h-3.5 w-3.5 ${trigger.isPending ? "animate-spin" : ""}`} />
          Sync now
        </Button>
        {detail.enabled ? (
          <Button size="sm" variant="outline" onClick={() => pause.mutate()} disabled={pause.isPending}>
            <Pause className="mr-1.5 h-3.5 w-3.5" /> Pause
          </Button>
        ) : (
          <Button size="sm" variant="outline" onClick={() => resume.mutate()} disabled={resume.isPending}>
            <Play className="mr-1.5 h-3.5 w-3.5" /> Resume
          </Button>
        )}
        <Button
          size="sm"
          variant="outline"
          className="ml-auto text-muted-foreground hover:text-destructive"
          disabled={del.isPending}
          onClick={() => {
            if (confirm(`Delete connector "${detail.name}"? This cannot be undone.`)) {
              del.mutate(detail.id, { onSuccess: onDeleted });
            }
          }}
        >
          <Trash2 className="mr-1.5 h-3.5 w-3.5" /> Delete
        </Button>
      </div>

      {/* Disabled / error banners */}
      {!detail.enabled && detail.disabled_reason && (
        <div className="flex items-center gap-2 rounded-md border border-muted-foreground/30 bg-muted px-3 py-2 text-sm text-muted-foreground">
          <Pause className="h-4 w-4 shrink-0" /> Paused: {detail.disabled_reason}
        </div>
      )}
      {detail.last_error && (
        <div className="flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <div className="min-w-0">
            <div className="font-medium">Last run failed</div>
            <div className="break-words font-mono text-xs opacity-90">{detail.last_error}</div>
          </div>
        </div>
      )}

      <SyncStatus detail={detail} />

      {/* Repositories */}
      {repos.length > 0 && (
        <Card>
          <CardHeader className="p-4 pb-2">
            <CardTitle className="text-sm font-semibold">
              Repositories ({repos.length})
            </CardTitle>
          </CardHeader>
          <CardContent className="p-4 pt-0">
            <div className="flex flex-wrap gap-1.5">
              {repos.map((r) => (
                <span
                  key={r}
                  className="rounded bg-muted px-2 py-0.5 font-mono text-xs text-muted-foreground"
                >
                  {r}
                </span>
              ))}
            </div>
          </CardContent>
        </Card>
      )}

      <SyncHistory detail={detail} />
    </div>
  );
}

function SyncStatus({ detail }: { detail: Detail }) {
  const stats = [
    { label: "Last run", value: fmtDate(detail.last_run_at) },
    { label: "Last success", value: fmtDate(detail.last_success_at) },
    {
      label: "Rows ingested",
      value: detail.last_rows_ingested != null ? detail.last_rows_ingested.toLocaleString() : "—",
    },
  ];
  return (
    <div className="grid grid-cols-3 gap-3">
      {stats.map((s) => (
        <Card key={s.label}>
          <CardContent className="p-4">
            <div className="text-xs text-muted-foreground">{s.label}</div>
            <div className="mt-1 truncate text-sm font-medium tabular-nums">{s.value}</div>
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

// Synthesize the latest run from detail fields. This is the seam for a future
// `/:id/runs` history endpoint — once it exists, fetch & map here instead.
function SyncHistory({ detail }: { detail: Detail }) {
  if (!detail.last_run_at) {
    return (
      <Card>
        <CardHeader className="p-4 pb-2">
          <CardTitle className="text-sm font-semibold">Sync history</CardTitle>
        </CardHeader>
        <CardContent className="p-4 pt-0 text-sm text-muted-foreground">
          No runs yet. Trigger a sync to ingest data.
        </CardContent>
      </Card>
    );
  }

  const ok = !detail.last_error;
  return (
    <Card>
      <CardHeader className="p-4 pb-2">
        <CardTitle className="text-sm font-semibold">Sync history</CardTitle>
      </CardHeader>
      <CardContent className="p-4 pt-0">
        <div className="flex items-center justify-between rounded-md border px-3 py-2 text-sm">
          <div className="flex items-center gap-2">
            <span
              className={`h-2 w-2 rounded-full ${ok ? "bg-emerald-500" : "bg-destructive"}`}
            />
            <span className="font-medium">{ok ? "Success" : "Failed"}</span>
            <span className="text-muted-foreground">{fmtDate(detail.last_run_at)}</span>
          </div>
          {detail.last_rows_ingested != null && (
            <span className="text-xs tabular-nums text-muted-foreground">
              {detail.last_rows_ingested.toLocaleString()} rows
            </span>
          )}
        </div>
      </CardContent>
    </Card>
  );
}
