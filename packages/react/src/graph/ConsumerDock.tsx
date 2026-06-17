/**
 * ConsumerDock — docked glass panel (right edge) listing the agents currently
 * reading/writing memory. Collapses to a thin rail (still showing who's active);
 * each row expands to reveal connection/process detail (transport, version,
 * host, ip, pid, subject, tenant, op count, last query).
 *
 * Mirrors the InspectorPanel container recipe and the memory firehose's status
 * language (violet=live / sky=polling / amber/idle=offline).
 */
import {
  ChevronLeft,
  ChevronRight,
  Pin,
  PinOff,
  Radio,
  WifiOff,
} from "lucide-react";
import { useGraphStore } from "./graph-store";
import { consumerColor, consumerIcon, verbLabel } from "./consumer-style";
import type { ConsumerStatus, LiveConsumer } from "./consumer-types";
import { cn } from "../internal/cn";

function ago(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 2) return "now";
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m}m` : `${Math.round(m / 60)}h`;
}

function clock(ts: number): string {
  try {
    return new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  } catch {
    return "";
  }
}

function StatusDot({ status }: { status: ConsumerStatus }) {
  if (status === "idle") {
    return (
      <span className="ky-flex ky-items-center ky-gap-1 ky-text-2xs ky-text-muted-foreground">
        <WifiOff className="ky-h-3 ky-w-3" /> offline
      </span>
    );
  }
  const cls = status === "live" ? "ky-bg-violet-400" : "ky-bg-sky-400";
  return (
    <span className="ky-flex ky-items-center ky-gap-1 ky-text-2xs ky-text-muted-foreground">
      <span className="ky-relative ky-flex ky-h-2 ky-w-2">
        <span
          className={cn(
            "ky-absolute ky-inline-flex ky-h-full ky-w-full ky-rounded-full ky-opacity-60",
            cls,
            status === "live" && "ky-animate-ping",
          )}
        />
        <span className={cn("ky-relative ky-inline-flex ky-h-2 ky-w-2 ky-rounded-full", cls)} />
      </span>
      {status === "live" ? "live" : "polling"}
    </span>
  );
}

/** One key/value line in the expanded detail block. */
function Detail({ k, v }: { k: string; v: string | null | undefined }) {
  if (!v) return null;
  return (
    <div className="ky-flex ky-items-baseline ky-gap-2">
      <span className="ky-w-16 ky-shrink-0 ky-text-2xs ky-uppercase ky-tracking-wide ky-text-muted-foreground/70">
        {k}
      </span>
      <span className="ky-min-w-0 ky-flex-1 ky-truncate ky-font-mono ky-text-2xs ky-text-foreground/90">
        {v}
      </span>
    </div>
  );
}

function ConsumerRow({ c }: { c: LiveConsumer }) {
  const hoverConsumer = useGraphStore((s) => s.hoverConsumer);
  const pinConsumer = useGraphStore((s) => s.pinConsumer);
  const expandConsumer = useGraphStore((s) => s.expandConsumer);
  const pinnedConsumerId = useGraphStore((s) => s.pinnedConsumerId);
  const expandedConsumerId = useGraphStore((s) => s.expandedConsumerId);
  const Icon = consumerIcon(c.kind);
  const color = consumerColor(c.kind);
  const pinned = c.id === pinnedConsumerId;
  const expanded = c.id === expandedConsumerId;
  const target =
    c.lastTargetCount > 0
      ? `${c.lastTargetCount} ${c.lastTargetCount === 1 ? "node" : "nodes"}`
      : "—";
  const realm = c.lastNamespaces[0];
  const ipPid = [c.ip, c.pid != null ? `pid ${c.pid}` : null].filter(Boolean).join(" · ") || null;
  // The default tenant is a nil UUID — show it as "default" rather than zeros.
  const tenantLabel = c.tenant
    ? c.tenant === "00000000-0000-0000-0000-000000000000"
      ? "default"
      : c.tenant
    : null;

  return (
    <li>
      <button
        type="button"
        onMouseEnter={() => hoverConsumer(c.id)}
        onMouseLeave={() => hoverConsumer(null)}
        onClick={() => expandConsumer(c.id)}
        className={cn(
          "ky-flex ky-w-full ky-items-center ky-gap-2.5 ky-rounded-md ky-px-2 ky-py-1.5 ky-text-left ky-transition-colors",
          expanded ? "ky-bg-accent" : "hover:ky-bg-accent/60",
          !c.active && "ky-opacity-45",
        )}
        style={{ boxShadow: `inset 2px 0 0 ${color}` }}
      >
        <Icon size={15} color={color} className="ky-shrink-0" />
        <div className="ky-min-w-0 ky-flex-1">
          <div className="ky-truncate ky-text-xs ky-font-medium ky-text-foreground">{c.label}</div>
          <div className="ky-truncate ky-text-2xs ky-text-muted-foreground">
            {c.lastVerb ? verbLabel(c.lastVerb) : "idle"} · {target}
            {realm ? ` · ${realm}` : ""}
          </div>
        </div>
        <time className="ky-shrink-0 ky-font-mono ky-text-2xs ky-tabular-nums ky-text-muted-foreground">
          {ago(c.lastTs)}
        </time>
        {expanded ? (
          <ChevronLeft className="ky-h-3 ky-w-3 ky-shrink-0 ky-rotate-90 ky-text-muted-foreground" />
        ) : (
          <ChevronRight className="ky-h-3 ky-w-3 ky-shrink-0 ky-text-muted-foreground/50" />
        )}
      </button>

      {expanded && (
        <div className="ky-mx-2 ky-mb-1 ky-mt-0.5 ky-space-y-1 ky-rounded-md ky-border ky-border-border/60 ky-bg-background/40 ky-p-2">
          <Detail k="Transport" v={c.transport} />
          <Detail k="Version" v={c.clientVersion} />
          <Detail k="Host" v={c.host} />
          <Detail k="Address" v={ipPid} />
          <Detail k="Subject" v={c.subject} />
          <Detail k="Tenant" v={tenantLabel} />
          <Detail
            k="Ops"
            v={`${c.opsCount} · since ${clock(c.firstTs)}`}
          />
          <Detail k="Query" v={c.lastQuery ? `"${c.lastQuery}"` : null} />
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              pinConsumer(c.id);
            }}
            className={cn(
              "ky-mt-1 ky-flex ky-items-center ky-gap-1.5 ky-rounded ky-border ky-px-2 ky-py-1 ky-text-2xs ky-transition-colors",
              pinned
                ? "ky-border-primary/60 ky-bg-primary/10 ky-text-primary"
                : "ky-border-border ky-text-muted-foreground hover:ky-text-foreground",
            )}
          >
            {pinned ? <PinOff className="ky-h-3 ky-w-3" /> : <Pin className="ky-h-3 ky-w-3" />}
            {pinned ? "Unpin" : "Pin — keep highlighted"}
          </button>
        </div>
      )}
    </li>
  );
}

/** Thin collapsed rail: who's active, at a glance, with an expand affordance. */
function CollapsedRail({ consumers, count }: { consumers: LiveConsumer[]; count: number }) {
  const toggleConsumerDock = useGraphStore((s) => s.toggleConsumerDock);
  const expandConsumer = useGraphStore((s) => s.expandConsumer);
  return (
    <div className="ky-flex ky-h-full ky-w-11 ky-shrink-0 ky-flex-col ky-items-center ky-gap-2 ky-border-l ky-border-border ky-glass ky-py-2 ky-animate-fade-in">
      <button
        type="button"
        onClick={toggleConsumerDock}
        title="Expand live consumers"
        className="ky-flex ky-h-7 ky-w-7 ky-items-center ky-justify-center ky-rounded-md ky-text-muted-foreground hover:ky-bg-accent hover:ky-text-foreground"
      >
        <ChevronLeft className="ky-h-4 ky-w-4" />
      </button>
      <div className="ky-relative">
        <Radio className="ky-h-4 ky-w-4 ky-text-primary" />
        {count > 0 && (
          <span className="ky-absolute -ky-right-2 -ky-top-2 ky-flex ky-h-3.5 ky-min-w-3.5 ky-items-center ky-justify-center ky-rounded-full ky-bg-primary ky-px-1 ky-text-[9px] ky-font-semibold ky-text-primary-foreground">
            {count}
          </span>
        )}
      </div>
      <div className="ky-mt-1 ky-flex ky-flex-col ky-items-center ky-gap-2 ky-overflow-y-auto">
        {consumers.map((c) => {
          const Icon = consumerIcon(c.kind);
          return (
            <button
              key={c.id}
              type="button"
              title={`${c.label} — ${c.lastVerb ? verbLabel(c.lastVerb) : "idle"}`}
              onClick={() => {
                toggleConsumerDock();
                expandConsumer(c.id);
              }}
              className={cn("ky-transition-opacity", !c.active && "ky-opacity-40")}
            >
              <Icon size={16} color={consumerColor(c.kind)} />
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function ConsumerDock() {
  const consumers = useGraphStore((s) => s.liveConsumers);
  const status = useGraphStore((s) => s.consumerStatus);
  const eventsPerMin = useGraphStore((s) => s.consumerEventsPerMin);
  const collapsed = useGraphStore((s) => s.consumerDockCollapsed);
  const toggleConsumerDock = useGraphStore((s) => s.toggleConsumerDock);

  // Newest activity first.
  const ordered = [...consumers].sort((a, b) => b.lastTs - a.lastTs);
  const activeCount = ordered.filter((c) => c.active).length;

  if (collapsed) {
    return <CollapsedRail consumers={ordered} count={activeCount} />;
  }

  return (
    <div className="ky-flex ky-h-full ky-w-72 ky-max-w-[85vw] ky-shrink-0 ky-flex-col ky-border-l ky-border-border ky-glass ky-animate-fade-in">
      <div className="ky-flex ky-items-center ky-justify-between ky-border-b ky-border-border ky-py-3 ky-pl-3 ky-pr-2">
        <div className="ky-flex ky-items-center ky-gap-2">
          <Radio className="ky-h-4 ky-w-4 ky-text-primary" />
          <span className="ky-text-sm ky-font-medium ky-text-foreground">Live consumers</span>
        </div>
        <div className="ky-flex ky-items-center ky-gap-2">
          <StatusDot status={status} />
          <button
            type="button"
            onClick={toggleConsumerDock}
            title="Collapse panel"
            className="ky-flex ky-h-6 ky-w-6 ky-items-center ky-justify-center ky-rounded ky-text-muted-foreground hover:ky-bg-accent hover:ky-text-foreground"
          >
            <ChevronRight className="ky-h-4 ky-w-4" />
          </button>
        </div>
      </div>

      {ordered.length === 0 ? (
        <div className="ky-flex ky-flex-1 ky-flex-col ky-items-center ky-justify-center ky-gap-2 ky-p-6 ky-text-center">
          <Radio className="ky-h-7 ky-w-7 ky-text-muted-foreground/50" />
          <div className="ky-text-xs ky-text-muted-foreground">No active consumers</div>
          <div className="ky-text-2xs ky-text-muted-foreground/70">
            Agents reading or writing memory will appear here in realtime.
          </div>
        </div>
      ) : (
        <ul className="ky-flex-1 ky-space-y-px ky-overflow-y-auto ky-p-2">
          {ordered.map((c) => (
            <ConsumerRow key={c.id} c={c} />
          ))}
        </ul>
      )}

      <div className="ky-border-t ky-border-border ky-px-3 ky-py-2 ky-text-2xs ky-text-muted-foreground">
        {activeCount} active · {eventsPerMin}/min
      </div>
    </div>
  );
}
