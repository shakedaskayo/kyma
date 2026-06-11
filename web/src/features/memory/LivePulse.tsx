import { useEffect, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Radio, WifiOff } from "lucide-react";
import { useReducedMotion } from "@/lib/motion";
import { relTime } from "@/lib/time";
import { cn } from "@/lib/utils";
import { kindStyle, sessionHue, STALL_THRESHOLD_MS } from "./lib";
import { useSharedMemoryStream } from "./stream-context";
import { type PulseEvent } from "./useMemoryStream";

/**
 * The live pulse rail — the heartbeat of the Memory surface. Firehose events
 * tick in one at a time (staggered slide+fade) with a session-colored thread
 * down the left edge so a conversation reads as one continuous strand. A live
 * dot pulses while the socket is connected; falls back gracefully to polling.
 */
export function LivePulse({ className }: { className?: string }) {
  const reduce = useReducedMotion();
  const { events, status, eventsPerMin, newestTs } = useSharedMemoryStream();

  // Re-tick relative timestamps every 5s.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 5000);
    return () => clearInterval(t);
  }, []);

  const connected = status === "live" || status === "polling";
  const stalled =
    connected && newestTs !== "" && now - new Date(newestTs).getTime() > STALL_THRESHOLD_MS;

  const fresh = events.filter((e) => !e.backfill);
  const history = events.filter((e) => e.backfill);

  return (
    <div className={cn("flex h-full flex-col", className)}>
      {/* Rail header */}
      <div className="flex items-center gap-2 px-1 pb-3">
        <span className="relative flex h-2 w-2">
          {connected && !reduce && (
            <span
              className={cn(
                "absolute inline-flex h-full w-full animate-ping rounded-full opacity-70",
                stalled ? "bg-amber-500" : status === "live" ? "bg-violet-500" : "bg-sky-500",
              )}
            />
          )}
          <span
            className={cn(
              "relative inline-flex h-2 w-2 rounded-full",
              stalled
                ? "bg-amber-400"
                : status === "live"
                  ? "bg-violet-400"
                  : status === "polling"
                    ? "bg-sky-400"
                    : "bg-muted-foreground",
            )}
          />
        </span>
        <span className="text-2xs font-medium uppercase tracking-[0.14em] text-foreground/80">
          Live pulse
        </span>
        <span className="ml-auto flex items-center gap-1 text-2xs text-muted-foreground">
          {connected ? (
            <Radio className="h-3 w-3" />
          ) : (
            <WifiOff className="h-3 w-3" />
          )}
          {stalled ? (
            <span
              className="text-amber-400"
              title="The stream is connected but no events are arriving. Check `kyma status` — capture may be failing."
            >
              capture stalled · last event {relTime(newestTs, now)}
            </span>
          ) : (
            <>
              {status === "live" ? "streaming" : status === "polling" ? "polling" : "idle"}
              {eventsPerMin > 0 && (
                <span className="tabular-nums text-foreground/70">· {eventsPerMin}/min</span>
              )}
            </>
          )}
        </span>
      </div>

      {/* Event stream */}
      <div className="relative min-h-0 flex-1 overflow-hidden">
        {/* Top fade so events appear to emerge from the header. */}
        <div className="pointer-events-none absolute inset-x-0 top-0 z-10 h-6 bg-gradient-to-b from-surface to-transparent" />
        <div className="h-full overflow-y-auto pr-1">
          {events.length === 0 ? (
            <PulseEmpty connected={connected} />
          ) : (
            <ul className="space-y-1.5">
              <AnimatePresence initial={false}>
                {fresh.map((ev) => (
                  <PulseRow key={ev.key} ev={ev} now={now} reduce={reduce} />
                ))}
              </AnimatePresence>
              {history.length > 0 && (
                <li className="flex items-center gap-2 px-1 pt-2 text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
                  <span className="h-px flex-1 bg-border/60" />
                  earlier
                  <span className="h-px flex-1 bg-border/60" />
                </li>
              )}
              {history.map((ev) => (
                <PulseRow key={ev.key} ev={ev} now={now} reduce={true} />
              ))}
            </ul>
          )}
        </div>
        {/* Bottom fade. */}
        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 h-6 bg-gradient-to-t from-surface to-transparent" />
      </div>
    </div>
  );
}

function PulseRow({ ev, now, reduce }: { ev: PulseEvent; now: number; reduce: boolean | null }) {
  const ks = kindStyle(ev.kind);
  const hue = sessionHue(ev.sessionId);
  return (
    <motion.li
      layout={!reduce}
      initial={reduce ? { opacity: 0 } : { opacity: 0, x: 14, height: 0 }}
      animate={reduce ? { opacity: 1 } : { opacity: 1, x: 0, height: "auto" }}
      exit={reduce ? { opacity: 0 } : { opacity: 0, x: -8 }}
      transition={{ duration: 0.24, ease: [0.16, 1, 0.3, 1] }}
      className="group relative overflow-hidden rounded-md border border-border/50 bg-card/40 pl-2.5 pr-2.5 py-2 transition-colors hover:border-border-strong/70 hover:bg-card/70"
      style={{ boxShadow: `inset 2px 0 0 hsl(${hue} 70% 60% / 0.85)` }}
    >
      <div className="flex items-center gap-2">
        <span className={cn("h-1.5 w-1.5 shrink-0 rounded-full", ks.dot)} />
        <span className="text-2xs font-medium text-foreground/90">{ks.label}</span>
        <span
          className="truncate font-mono text-[10px]"
          style={{ color: `hsl(${hue} 55% 70%)` }}
          title={ev.sessionId}
        >
          {ev.sessionId.slice(0, 10)}
        </span>
        <span className="ml-auto shrink-0 text-[10px] tabular-nums text-muted-foreground">
          {relTime(ev.ts, now)}
        </span>
      </div>
      {ev.text && (
        <p className="mt-1 line-clamp-2 text-xs leading-relaxed text-foreground/75">{ev.text}</p>
      )}
    </motion.li>
  );
}

function PulseEmpty({ connected }: { connected: boolean }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-4 py-10 text-center">
      <span className="relative flex h-8 w-8 items-center justify-center">
        <span className="absolute inline-flex h-full w-full rounded-full bg-violet-500/10" />
        <Radio className="h-4 w-4 text-violet-400/70" />
      </span>
      <p className="text-xs font-medium text-foreground/70">
        {connected ? "Listening for activity" : "Connecting…"}
      </p>
      <p className="max-w-[18rem] text-2xs leading-relaxed text-muted-foreground">
        Activity from connected coding agents will stream in here as it arrives.
      </p>
    </div>
  );
}
