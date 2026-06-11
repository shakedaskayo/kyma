import { useEffect, useRef, useState } from "react";
import { LiveSession, type LiveStatus } from "@/sdk/discover-live";
import { useSession } from "@/sdk/session";
import { str, isBackfill, spanRowToEvent } from "./lib";

/**
 * A single firehose event in the live pulse. `key` is stable per event so
 * AnimatePresence can track entrance/exit; `ts` is the source timestamp.
 * `backfill` is true when the event was already old at arrival (suppresses
 * the live-flash animation — backfill history renders without it).
 */
export interface PulseEvent {
  key: string;
  ts: string;
  kind: string;
  sessionId: string;
  realm: string;
  text: string;
  backfill: boolean;
}

/** Shape produced by rowToEvent / spanRowToEvent before backfill classification. */
type RawEvent = Omit<PulseEvent, "backfill">;

export type StreamMode = "live" | "polling" | "idle";

const FIREHOSE_SOURCE = "default.claude_code_events";
const FIREHOSE_DB = "default";
const OPS_SOURCE = "otel.otel_traces";
const OPS_DB = "otel";
const OPS_KQL =
  "otel_traces | where name startswith 'memory.' or name startswith 'agent.' or name startswith 'ingest.' | sort by start_time desc | take 12";
const MAX_EVENTS = 60; // ring cap kept in state
const RELEASE_INTERVAL_MS = 900; // visual cadence — ~1 entry/sec sliding in
const POLL_INTERVAL_MS = 2000;

function rowToEvent(row: Record<string, unknown>): RawEvent {
  const ts = str(row.ts) || str(row.at) || str(row.timestamp) || new Date().toISOString();
  const kind = str(row.kind) || "event";
  const sessionId = str(row.session_id) || "—";
  const realm = str(row.realm);
  const text =
    str(row.text) ||
    (row.tool_name ? `tool: ${str(row.tool_name)}` : "") ||
    str(row.content) ||
    "";
  // Stable-ish key: timestamp + session + a short text fingerprint.
  const key = `${ts}|${sessionId}|${text.slice(0, 24)}`;
  return { key, ts, kind, sessionId, realm, text };
}

/**
 * Live-streams firehose events from `default.claude_code_events` and op spans
 * from `otel.otel_traces`.
 *
 * Primary path: the explore live-tail WebSocket (`LiveSession`) — ~250ms
 * latency row frames. If the socket errors (or never reaches `live`), we fall
 * back to fast incremental polling against the same source via the explore
 * search endpoint, fetching rows newer than a watermark.
 *
 * In BOTH paths incoming events are buffered and *released one at a time* on a
 * ~1/sec cadence so the pulse rail reads as a living stream rather than a batch
 * dump. `reducedMotion` disables the staged release (everything appears at once).
 *
 * Events that are already older than BACKFILL_THRESHOLD_MS at arrival are
 * classified as `backfill: true` and rendered immediately into history without
 * the live-flash animation.
 */
export function useMemoryStream(opts?: { reducedMotion?: boolean; enabled?: boolean }): {
  events: PulseEvent[];
  status: StreamMode;
  liveStatus: LiveStatus | null;
  eventsPerMin: number;
  newestTs: string;
} {
  const { endpoint, token } = useSession();
  const enabled = opts?.enabled ?? true;
  const staged = !opts?.reducedMotion;

  const [events, setEvents] = useState<PulseEvent[]>([]);
  const [status, setStatus] = useState<StreamMode>("idle");
  const [liveStatus, setLiveStatus] = useState<LiveStatus | null>(null);

  // Buffer of not-yet-released events (FIFO), drained by the release timer.
  const bufferRef = useRef<PulseEvent[]>([]);
  const seenRef = useRef<Set<string>>(new Set());
  // Rolling window of release timestamps for events/min.
  const recentTimesRef = useRef<number[]>([]);
  const [eventsPerMin, setEventsPerMin] = useState(0);

  // Pull the whole buffer into state at once (reduced-motion path).
  function flushAll() {
    if (bufferRef.current.length === 0) return;
    const batch = bufferRef.current;
    bufferRef.current = [];
    const now = Date.now();
    for (let i = 0; i < batch.length; i++) recentTimesRef.current.push(now);
    setEvents((cur) => [...batch].reverse().concat(cur).slice(0, MAX_EVENTS));
  }

  // Ingest a batch of raw rows: classify backfill at arrival, dedupe, push
  // novel live events into the staged buffer and backfill history directly.
  const ingest = useRef(
    (
      rows: Record<string, unknown>[],
      toEvent: (r: Record<string, unknown>) => RawEvent = rowToEvent,
    ) => {
      const arrival = Date.now();
      const freshLive: PulseEvent[] = [];
      const history: PulseEvent[] = [];
      for (const r of rows) {
        const ev = toEvent(r);
        if (seenRef.current.has(ev.key)) continue;
        seenRef.current.add(ev.key);
        const backfill = isBackfill(ev.ts, arrival);
        (backfill ? history : freshLive).push({ ...ev, backfill });
      }
      // History renders immediately without animation (old rows must not animate as live).
      if (history.length > 0) {
        history.sort((a, b) => b.ts.localeCompare(a.ts));
        setEvents((cur) => [...cur, ...history].slice(0, MAX_EVENTS));
      }
      if (freshLive.length === 0) return;
      // Oldest first into the buffer so they tick in chronologically.
      freshLive.sort((a, b) => a.ts.localeCompare(b.ts));
      bufferRef.current.push(...freshLive);
      if (!staged) {
        // Flush immediately (reduced motion): no staged release.
        flushAll();
      }
    },
  );

  // Staged release: emit one buffered event per tick.
  useEffect(() => {
    if (!staged) return;
    const t = setInterval(() => {
      const next = bufferRef.current.shift();
      if (!next) return;
      recentTimesRef.current.push(Date.now());
      setEvents((cur) => [next, ...cur].slice(0, MAX_EVENTS));
    }, RELEASE_INTERVAL_MS);
    return () => clearInterval(t);
  }, [staged]);

  // events/min from the rolling release-time window (recomputed every 5s).
  useEffect(() => {
    const t = setInterval(() => {
      const cutoff = Date.now() - 60_000;
      recentTimesRef.current = recentTimesRef.current.filter((x) => x >= cutoff);
      setEventsPerMin(recentTimesRef.current.length);
    }, 5000);
    return () => clearInterval(t);
  }, []);

  // The live socket, with a polling fallback armed if it never goes live.
  useEffect(() => {
    if (!enabled || !endpoint || !token) {
      setStatus("idle");
      return;
    }

    let disposed = false;
    let session: LiveSession | null = null;
    let opsSession: LiveSession | null = null;
    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let fallbackTimer: ReturnType<typeof setTimeout> | null = null;
    let usingPolling = false;

    const startPolling = () => {
      if (usingPolling || disposed) return;
      usingPolling = true;
      session?.close();
      opsSession?.close();
      session = null;
      opsSession = null;
      setStatus("polling");
      const tick = async () => {
        try {
          // Poll the firehose directly via the NDJSON `/v1/query` surface
          // (one row per line). We sort newest-first and cap so we always pick
          // up the freshest events; the `seen` dedupe in `ingest` makes the
          // overlapping window cheap. (The explore live-tail is the primary
          // path; this is the resilient fallback.)
          const kql = "claude_code_events | sort by ts desc | take 20";
          const res = await fetch(`${endpoint.replace(/\/$/, "")}/v1/query`, {
            method: "POST",
            headers: {
              authorization: `Bearer ${token}`,
              "content-type": "application/x-kql",
              "x-database": FIREHOSE_DB,
            },
            body: kql,
          });
          if (!res.ok || disposed) return;
          const text = await res.text();
          const rows: Record<string, unknown>[] = [];
          for (const line of text.split("\n")) {
            const t = line.trim();
            if (!t) continue;
            try {
              rows.push(JSON.parse(t) as Record<string, unknown>);
            } catch {
              /* skip non-row lines */
            }
          }
          if (rows.length > 0) ingest.current(rows);
        } catch {
          /* transient — keep polling */
        }

        // ops fetch in polling mode
        try {
          const opsRes = await fetch(`${endpoint.replace(/\/$/, "")}/v1/query`, {
            method: "POST",
            headers: {
              authorization: `Bearer ${token}`,
              "content-type": "application/x-kql",
              "x-database": OPS_DB,
            },
            body: OPS_KQL + "\n| take 20",
          });
          if (opsRes.ok && !disposed) {
            const opsRows: Record<string, unknown>[] = [];
            for (const line of (await opsRes.text()).split("\n")) {
              const t = line.trim();
              if (t) {
                try {
                  opsRows.push(JSON.parse(t));
                } catch {
                  /* skip */
                }
              }
            }
            if (opsRows.length) ingest.current(opsRows, spanRowToEvent);
          }
        } catch {
          /* ops polling is best-effort */
        }
      };
      void tick();
      pollTimer = setInterval(() => void tick(), POLL_INTERVAL_MS);
    };

    // Arm fallback: if we don't reach "live" within 4s, switch to polling.
    fallbackTimer = setTimeout(() => startPolling(), 4000);

    session = new LiveSession({
      endpoint,
      token,
      body: {
        query: "claude_code_events | sort by ts desc | take 12",
        scope: { kind: "sources", sources: [FIREHOSE_SOURCE] },
        time_range: null,
      },
      onStatus: (s) => {
        if (disposed) return;
        setLiveStatus(s);
        if (s === "live" || s === "backfilling") {
          if (fallbackTimer) {
            clearTimeout(fallbackTimer);
            fallbackTimer = null;
          }
          if (!usingPolling) setStatus("live");
        }
        if (s === "error") startPolling();
      },
      onFrame: (f) => {
        if (disposed || usingPolling) return;
        if (f.type === "rows" && Array.isArray(f.rows)) ingest.current(f.rows);
      },
    });

    opsSession = new LiveSession({
      endpoint,
      token,
      body: {
        query: OPS_KQL,
        scope: { kind: "sources", sources: [OPS_SOURCE] },
        time_range: null,
      },
      onStatus: () => {},
      onFrame: (f) => {
        if (disposed || usingPolling) return;
        if (f.type === "rows" && Array.isArray(f.rows)) ingest.current(f.rows, spanRowToEvent);
      },
    });

    return () => {
      disposed = true;
      if (fallbackTimer) clearTimeout(fallbackTimer);
      if (pollTimer) clearInterval(pollTimer);
      session?.close();
      opsSession?.close();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [endpoint, token, enabled]);

  const newestTs = events.reduce<string>((acc, e) => (e.ts > acc ? e.ts : acc), "");

  return { events, status, liveStatus, eventsPerMin, newestTs };
}
