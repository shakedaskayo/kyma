/**
 * ConsumerDock — docked glass panel (right edge) listing the agents currently
 * reading/writing memory, mirroring the InspectorPanel container recipe and the
 * memory firehose's status language (violet=live / sky=polling / amber=stalled).
 *
 * Rows hover → emphasise that consumer's beams on the canvas; click → pin
 * ("watch this agent"), keeping its rim marker visible past idle.
 */
import { Radio, WifiOff } from "lucide-react";
import { useGraphStore } from "./graph-store";
import { consumerColor, consumerIcon, verbLabel } from "./consumer-style";
import type { LiveConsumer } from "./consumer-types";
import { cn } from "../internal/cn";

function ago(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 2) return "now";
  if (s < 60) return `${s}s`;
  const m = Math.round(s / 60);
  return m < 60 ? `${m}m` : `${Math.round(m / 60)}h`;
}

function StatusDot({ status }: { status: "live" | "polling" | "idle" }) {
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

function ConsumerRow({ c }: { c: LiveConsumer }) {
  const hoverConsumer = useGraphStore((s) => s.hoverConsumer);
  const pinConsumer = useGraphStore((s) => s.pinConsumer);
  const pinnedConsumerId = useGraphStore((s) => s.pinnedConsumerId);
  const Icon = consumerIcon(c.kind);
  const color = consumerColor(c.kind);
  const pinned = c.id === pinnedConsumerId;
  const target =
    c.lastTargetCount > 0
      ? `${c.lastTargetCount} ${c.lastTargetCount === 1 ? "node" : "nodes"}`
      : "—";
  const realm = c.lastNamespaces[0];

  return (
    <li>
      <button
        type="button"
        onMouseEnter={() => hoverConsumer(c.id)}
        onMouseLeave={() => hoverConsumer(null)}
        onClick={() => pinConsumer(c.id)}
        title={pinned ? "Unpin" : "Pin — keep this agent highlighted"}
        className={cn(
          "ky-flex ky-w-full ky-items-center ky-gap-2.5 ky-rounded-md ky-px-2 ky-py-1.5 ky-text-left ky-transition-colors",
          pinned ? "ky-bg-accent" : "hover:ky-bg-accent/60",
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
      </button>
    </li>
  );
}

export function ConsumerDock() {
  const consumers = useGraphStore((s) => s.liveConsumers);
  const status = useGraphStore((s) => s.consumerStatus);
  const eventsPerMin = useGraphStore((s) => s.consumerEventsPerMin);

  // Newest activity first.
  const ordered = [...consumers].sort((a, b) => b.lastTs - a.lastTs);

  return (
    <div className="ky-flex ky-h-full ky-w-72 ky-max-w-[85vw] ky-shrink-0 ky-flex-col ky-border-l ky-border-border ky-glass ky-animate-fade-in">
      <div className="ky-flex ky-items-center ky-justify-between ky-border-b ky-border-border ky-p-3">
        <div className="ky-flex ky-items-center ky-gap-2">
          <Radio className="ky-h-4 ky-w-4 ky-text-primary" />
          <span className="ky-text-sm ky-font-medium ky-text-foreground">Live consumers</span>
        </div>
        <StatusDot status={status} />
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
        {ordered.length} active · {eventsPerMin}/min
      </div>
    </div>
  );
}
