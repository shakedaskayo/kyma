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
      <span className="pv-flex pv-items-center pv-gap-1 pv-text-2xs pv-text-muted-foreground">
        <WifiOff className="pv-h-3 pv-w-3" /> offline
      </span>
    );
  }
  const cls = status === "live" ? "pv-bg-violet-400" : "pv-bg-sky-400";
  return (
    <span className="pv-flex pv-items-center pv-gap-1 pv-text-2xs pv-text-muted-foreground">
      <span className="pv-relative pv-flex pv-h-2 pv-w-2">
        <span
          className={cn(
            "pv-absolute pv-inline-flex pv-h-full pv-w-full pv-rounded-full pv-opacity-60",
            cls,
            status === "live" && "pv-animate-ping",
          )}
        />
        <span className={cn("pv-relative pv-inline-flex pv-h-2 pv-w-2 pv-rounded-full", cls)} />
      </span>
      {status === "live" ? "live" : "polling"}
    </span>
  );
}

/** One key/value line in the expanded detail block. */
function Detail({ k, v }: { k: string; v: string | null | undefined }) {
  if (!v) return null;
  return (
    <div className="pv-flex pv-items-baseline pv-gap-2">
      <span className="pv-w-16 pv-shrink-0 pv-text-2xs pv-uppercase pv-tracking-wide pv-text-muted-foreground/70">
        {k}
      </span>
      <span className="pv-min-w-0 pv-flex-1 pv-truncate pv-font-mono pv-text-2xs pv-text-foreground/90">
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
          "pv-flex pv-w-full pv-items-center pv-gap-2.5 pv-rounded-md pv-px-2 pv-py-1.5 pv-text-left pv-transition-colors",
          expanded ? "pv-bg-accent" : "hover:pv-bg-accent/60",
          !c.active && "pv-opacity-45",
        )}
        style={{ boxShadow: `inset 2px 0 0 ${color}` }}
      >
        <Icon size={15} color={color} className="pv-shrink-0" />
        <div className="pv-min-w-0 pv-flex-1">
          <div className="pv-truncate pv-text-xs pv-font-medium pv-text-foreground">{c.label}</div>
          <div className="pv-truncate pv-text-2xs pv-text-muted-foreground">
            {c.lastVerb ? verbLabel(c.lastVerb) : "idle"} · {target}
            {realm ? ` · ${realm}` : ""}
          </div>
        </div>
        <time className="pv-shrink-0 pv-font-mono pv-text-2xs pv-tabular-nums pv-text-muted-foreground">
          {ago(c.lastTs)}
        </time>
        {expanded ? (
          <ChevronLeft className="pv-h-3 pv-w-3 pv-shrink-0 pv-rotate-90 pv-text-muted-foreground" />
        ) : (
          <ChevronRight className="pv-h-3 pv-w-3 pv-shrink-0 pv-text-muted-foreground/50" />
        )}
      </button>

      {expanded && (
        <div className="pv-mx-2 pv-mb-1 pv-mt-0.5 pv-space-y-1 pv-rounded-md pv-border pv-border-border/60 pv-bg-background/40 pv-p-2">
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
              "pv-mt-1 pv-flex pv-items-center pv-gap-1.5 pv-rounded pv-border pv-px-2 pv-py-1 pv-text-2xs pv-transition-colors",
              pinned
                ? "pv-border-primary/60 pv-bg-primary/10 pv-text-primary"
                : "pv-border-border pv-text-muted-foreground hover:pv-text-foreground",
            )}
          >
            {pinned ? <PinOff className="pv-h-3 pv-w-3" /> : <Pin className="pv-h-3 pv-w-3" />}
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
    <div className="pv-flex pv-h-full pv-w-11 pv-shrink-0 pv-flex-col pv-items-center pv-gap-2 pv-border-l pv-border-border pv-glass pv-py-2 pv-animate-fade-in">
      <button
        type="button"
        onClick={toggleConsumerDock}
        title="Expand live consumers"
        className="pv-flex pv-h-7 pv-w-7 pv-items-center pv-justify-center pv-rounded-md pv-text-muted-foreground hover:pv-bg-accent hover:pv-text-foreground"
      >
        <ChevronLeft className="pv-h-4 pv-w-4" />
      </button>
      <div className="pv-relative">
        <Radio className="pv-h-4 pv-w-4 pv-text-primary" />
        {count > 0 && (
          <span className="pv-absolute -pv-right-2 -pv-top-2 pv-flex pv-h-3.5 pv-min-w-3.5 pv-items-center pv-justify-center pv-rounded-full pv-bg-primary pv-px-1 pv-text-[9px] pv-font-semibold pv-text-primary-foreground">
            {count}
          </span>
        )}
      </div>
      <div className="pv-mt-1 pv-flex pv-flex-col pv-items-center pv-gap-2 pv-overflow-y-auto">
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
              className={cn("pv-transition-opacity", !c.active && "pv-opacity-40")}
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
    <div className="pv-flex pv-h-full pv-w-72 pv-max-w-[85vw] pv-shrink-0 pv-flex-col pv-border-l pv-border-border pv-glass pv-animate-fade-in">
      <div className="pv-flex pv-items-center pv-justify-between pv-border-b pv-border-border pv-py-3 pv-pl-3 pv-pr-2">
        <div className="pv-flex pv-items-center pv-gap-2">
          <Radio className="pv-h-4 pv-w-4 pv-text-primary" />
          <span className="pv-text-sm pv-font-medium pv-text-foreground">Live consumers</span>
        </div>
        <div className="pv-flex pv-items-center pv-gap-2">
          <StatusDot status={status} />
          <button
            type="button"
            onClick={toggleConsumerDock}
            title="Collapse panel"
            className="pv-flex pv-h-6 pv-w-6 pv-items-center pv-justify-center pv-rounded pv-text-muted-foreground hover:pv-bg-accent hover:pv-text-foreground"
          >
            <ChevronRight className="pv-h-4 pv-w-4" />
          </button>
        </div>
      </div>

      {ordered.length === 0 ? (
        <div className="pv-flex pv-flex-1 pv-flex-col pv-items-center pv-justify-center pv-gap-2 pv-p-6 pv-text-center">
          <Radio className="pv-h-7 pv-w-7 pv-text-muted-foreground/50" />
          <div className="pv-text-xs pv-text-muted-foreground">No active consumers</div>
          <div className="pv-text-2xs pv-text-muted-foreground/70">
            Agents reading or writing memory will appear here in realtime.
          </div>
        </div>
      ) : (
        <ul className="pv-flex-1 pv-space-y-px pv-overflow-y-auto pv-p-2">
          {ordered.map((c) => (
            <ConsumerRow key={c.id} c={c} />
          ))}
        </ul>
      )}

      <div className="pv-border-t pv-border-border pv-px-3 pv-py-2 pv-text-2xs pv-text-muted-foreground">
        {activeCount} active · {eventsPerMin}/min
      </div>
    </div>
  );
}
