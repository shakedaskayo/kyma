import { useEffect } from "react";
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
import { useQuery } from "@tanstack/react-query";
import { useSession } from "@/sdk/session";
import { getStats } from "@/sdk/graph";
import {
  deriveStatus,
  type CatalogEntry,
  type ConnectorDetail as Detail,
} from "@/sdk/connectors";
import { useCredentials } from "@/features/credentials/useCredentials";
import { formatInterval, graphNameForEntry } from "./connector-kinds";
import { BrandIcon } from "./BrandIcon";
import { StatusBadge } from "./StatusBadge";
import { useOAuthConnect } from "./useOAuthConnect";
import {
  useConnectorCatalog,
  useDeleteConnector,
  usePatchConnector,
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
  const { data: catalog } = useConnectorCatalog();
  const entry = catalog?.find((e) => e.type_id === detail.type);
  const status = deriveStatus(detail);

  const graphName = graphNameForEntry(entry, detail.type);
  const resourceLabel = entry?.resource?.label ?? "Resources";
  const resourceKey = entry?.resource?.config_key;
  const resources =
    (resourceKey ? (detail.config?.[resourceKey] as string[] | undefined) : undefined) ?? [];

  const openGraph = () => {
    // Pass the composite `${database}/${graph}` key as a URL search param so
    // the graph route can seed focusQuery on mount (one-way URL → graph focus).
    if (graphName) {
      void navigate({ to: "/graph", search: { graph: `${detail.target_database}/${graphName}` } });
    } else {
      void navigate({ to: "/graph" });
    }
  };

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      {/* Header */}
      <div className="flex items-start gap-4">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border bg-background">
          <BrandIcon brand={entry?.brand ?? ""} size={22} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-lg font-semibold">{detail.name}</h1>
            <StatusBadge status={status} />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
            <span>{entry?.label ?? detail.type}</span>
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
        {entry?.graph_name && (
          <Button variant="outline" size="sm" onClick={openGraph}>
            <Network className="mr-1.5 h-3.5 w-3.5" /> Open graph
          </Button>
        )}
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

      <SyncStatus detail={detail} graphName={entry?.graph_name ?? null} />

      <ConnectorCredentialCard detail={detail} entry={entry} />

      {/* Selected resources (e.g. repositories) */}
      {resources.length > 0 && (
        <Card>
          <CardHeader className="p-4 pb-2">
            <CardTitle className="text-sm font-semibold">
              {resourceLabel} ({resources.length})
            </CardTitle>
          </CardHeader>
          <CardContent className="p-4 pt-0">
            <div className="flex flex-wrap gap-1.5">
              {resources.map((r) => (
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

/** Shows the connector's linked credential and, for OAuth connectors, a
 * Reconnect action that re-runs the connect flow and repoints the connector at
 * the freshly minted credential. */
function ConnectorCredentialCard({ detail, entry }: { detail: Detail; entry?: CatalogEntry }) {
  const credentialId =
    typeof detail.config?.credential_id === "string"
      ? (detail.config.credential_id as string)
      : undefined;
  const { data: creds } = useCredentials();
  const patch = usePatchConnector(detail.id);
  const isOAuthConn = entry?.auth_mode === "oauth";
  const oauth = useOAuthConnect(entry?.oauth_provider, detail.type);

  // On a successful reconnect, repoint the connector at the new credential.
  useEffect(() => {
    if (oauth.phase === "success" && oauth.account) {
      patch.mutate({ config: { ...detail.config, credential_id: oauth.account.credentialId } });
      oauth.reset();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [oauth.phase, oauth.account]);

  if (!credentialId && !isOAuthConn) return null;
  const cred = creds?.find((c) => c.id === credentialId);
  const connecting = oauth.phase === "starting" || oauth.phase === "awaiting";

  return (
    <Card>
      <CardHeader className="p-4 pb-2">
        <CardTitle className="text-sm font-semibold">Credential</CardTitle>
      </CardHeader>
      <CardContent className="p-4 pt-0">
        {cred ? (
          <div className="flex items-center gap-3">
            <BrandIcon brand={entry?.brand ?? ""} size={18} />
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm font-medium">{cred.label}</div>
              <div className="font-mono text-xs text-muted-foreground">
                {cred.kind} · {cred.preview}
              </div>
            </div>
            {isOAuthConn && (
              <Button
                variant="outline"
                size="sm"
                disabled={connecting}
                onClick={() => oauth.connect({ scopes: entry?.oauth_scopes })}
              >
                {connecting ? "Waiting…" : "Reconnect"}
              </Button>
            )}
          </div>
        ) : (
          <div className="flex items-center justify-between text-sm text-muted-foreground">
            <span>{credentialId ? "Linked credential not found." : "No credential linked."}</span>
            {isOAuthConn && (
              <Button
                variant="outline"
                size="sm"
                disabled={connecting}
                onClick={() => oauth.connect({ scopes: entry?.oauth_scopes })}
              >
                {connecting ? "Waiting…" : "Connect"}
              </Button>
            )}
          </div>
        )}
        {oauth.phase === "error" && oauth.error && (
          <p className="mt-2 text-xs text-destructive">{oauth.error}</p>
        )}
        {oauth.popupBlocked && oauth.authorizeUrl && (
          <p className="mt-2 text-xs text-amber-600">
            <a className="underline" href={oauth.authorizeUrl} target="_blank" rel="noreferrer">
              Open authorization in a new tab
            </a>
          </p>
        )}
      </CardContent>
    </Card>
  );
}

function SyncStatus({ detail, graphName }: { detail: Detail; graphName: string | null }) {
  // For graph connectors, show the connector's *graph size* (the meaningful,
  // cumulative number) rather than last-run rows — which sit at 0 once polling
  // reaches steady state. Non-graph connectors show last-run rows instead.
  const { endpoint, token, database } = useSession();
  const { data: gstats } = useQuery({
    queryKey: ["graph", "stats", endpoint, database, graphName],
    queryFn: () => getStats({ endpoint, token, database, graph: graphName! }),
    enabled: Boolean(endpoint && graphName),
  });
  const stats = graphName
    ? [
        { label: "Last sync", value: fmtDate(detail.last_success_at) },
        { label: "Nodes", value: gstats ? gstats.total_nodes.toLocaleString() : "—" },
        { label: "Edges", value: gstats ? gstats.total_relationships.toLocaleString() : "—" },
      ]
    : [
        { label: "Last sync", value: fmtDate(detail.last_success_at) },
        {
          label: "Rows (last run)",
          value: detail.last_rows_ingested != null ? detail.last_rows_ingested.toLocaleString() : "—",
        },
        { label: "Interval", value: formatInterval(detail.schedule_ms) },
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
