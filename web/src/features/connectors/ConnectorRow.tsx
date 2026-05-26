import { useNavigate } from "@tanstack/react-router";
import { Pause, Play, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { deriveStatus, type ConnectorSummary } from "@/sdk/connectors";
import { kindByType } from "./connector-kinds";
import {
  useConnector,
  useDeleteConnector,
  usePauseConnector,
  useResumeConnector,
  useTriggerConnector,
} from "./useConnectors";
import { StatusBadge } from "./StatusBadge";

function relTime(iso: string | null): string {
  if (!iso) return "never";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "never";
  const diff = Date.now() - then;
  const s = Math.round(diff / 1000);
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

export function ConnectorRow({ connector }: { connector: ConnectorSummary }) {
  const navigate = useNavigate();
  // Lazy per-row hydration — the list endpoint carries no status/metrics.
  const { data: detail } = useConnector(connector.id);
  const trigger = useTriggerConnector(connector.id);
  const pause = usePauseConnector(connector.id);
  const resume = useResumeConnector(connector.id);
  const del = useDeleteConnector();

  const kind = kindByType(connector.type);
  const Icon = kind?.icon;
  const status = detail ? deriveStatus(detail) : connector.enabled ? "idle" : "disabled";

  const open = () => navigate({ to: "/connectors/$id", params: { id: connector.id } });

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={open}
      onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && open()}
      className="group flex items-center gap-4 rounded-lg border bg-background px-4 py-3 text-left shadow-sm transition hover:border-primary/40 hover:shadow-md"
    >
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-primary/10 text-primary/80">
        {Icon ? <Icon className="h-4 w-4" /> : null}
      </div>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-medium text-foreground">{connector.name}</span>
          <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-muted-foreground">
            {kind?.label ?? connector.type}
          </span>
        </div>
        <div className="mt-0.5 flex items-center gap-3 text-xs text-muted-foreground">
          <span>Last sync {relTime(detail?.last_success_at ?? null)}</span>
          {detail?.last_rows_ingested != null && (
            <span className="tabular-nums">{detail.last_rows_ingested.toLocaleString()} rows</span>
          )}
        </div>
      </div>

      <StatusBadge status={status} />

      {/* Quick actions — stopPropagation so they don't open the detail view. */}
      <div
        className="flex items-center gap-0.5 opacity-0 transition group-hover:opacity-100"
        onClick={(e) => e.stopPropagation()}
      >
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8"
          title="Sync now"
          disabled={trigger.isPending || !connector.enabled}
          onClick={() => trigger.mutate()}
        >
          <RefreshCw className={trigger.isPending ? "animate-spin" : ""} />
        </Button>
        {connector.enabled ? (
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            title="Pause"
            disabled={pause.isPending}
            onClick={() => pause.mutate()}
          >
            <Pause />
          </Button>
        ) : (
          <Button
            variant="ghost"
            size="icon"
            className="h-8 w-8"
            title="Resume"
            disabled={resume.isPending}
            onClick={() => resume.mutate()}
          >
            <Play />
          </Button>
        )}
        <Button
          variant="ghost"
          size="icon"
          className="h-8 w-8 text-muted-foreground hover:text-destructive"
          title="Delete"
          disabled={del.isPending}
          onClick={() => {
            if (confirm(`Delete connector "${connector.name}"? This cannot be undone.`)) {
              del.mutate(connector.id);
            }
          }}
        >
          <Trash2 />
        </Button>
      </div>
    </div>
  );
}
