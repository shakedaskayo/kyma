/**
 * useConsumerStream — streams live consumer activity from `/v1/consumers/live`
 * and shapes it into the `LiveConsumersData` snapshot the SDK graph renders.
 *
 * Owns the WebSocket (so the feature stays in web/, the SDK stays socket-free).
 * Modeled on the memory firehose hook: dedup, a rolling events/min window, an
 * idle sweep that fades then drops stale consumers, and beam construction that
 * respects reduced motion. When `enabled` flips false the socket is torn down.
 */
import { useEffect, useMemo, useRef, useState } from "react";
import type { ConsumerBeam, ConsumerStatus, LiveConsumer, LiveConsumersData } from "@kyma-ai/react/graph";
import { useSession } from "@/sdk/session";
import { ConsumerLiveSession, type ConsumerActivityFrame } from "./consumer-live";
import { ACTIVE_MS, DROP_MS, MAX_BEAMS, SWEEP_MS, dedupKey, toBeams, toConsumer } from "./consumer-lib";

function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

export function useConsumerStream(opts?: {
  enabled?: boolean;
  reducedMotion?: boolean;
}): LiveConsumersData {
  const enabled = opts?.enabled ?? true;
  const endpoint = useSession((s) => s.endpoint);
  const token = useSession((s) => s.token);
  const reduced = opts?.reducedMotion ?? prefersReducedMotion();

  const [consumers, setConsumers] = useState<LiveConsumer[]>([]);
  const [beams, setBeams] = useState<ConsumerBeam[]>([]);
  const [status, setStatus] = useState<ConsumerStatus>("idle");
  const [eventsPerMin, setEventsPerMin] = useState(0);

  const consumersRef = useRef<Map<string, LiveConsumer>>(new Map());
  const beamsRef = useRef<ConsumerBeam[]>([]);
  const seenRef = useRef<Set<string>>(new Set());
  const eventTimesRef = useRef<number[]>([]);

  // Stable ingest fn (reads `reduced` via a ref so the WS effect needn't restart).
  const reducedRef = useRef(reduced);
  reducedRef.current = reduced;

  const ingest = useRef((a: ConsumerActivityFrame, backfill: boolean) => {
    const key = dedupKey(a);
    if (seenRef.current.has(key)) return;
    seenRef.current.add(key);

    const now = Date.now();
    // Backfill rows are historic — old at arrival, so they enter the list
    // already inactive (faded) and never spawn beams.
    const active = !backfill && now - a.ts < ACTIVE_MS;
    consumersRef.current.set(a.consumer_id, toConsumer(a, active));
    setConsumers([...consumersRef.current.values()]);

    if (!backfill) {
      eventTimesRef.current.push(now);
      if (!reducedRef.current && a.node_ids.length > 0) {
        const fresh = toBeams(a, now);
        beamsRef.current = [...beamsRef.current, ...fresh].slice(-MAX_BEAMS);
        setBeams(beamsRef.current);
      }
    }
  }).current;

  // Idle sweep: fade → drop stale consumers, prune expired beams, recompute rate.
  useEffect(() => {
    const t = setInterval(() => {
      const now = Date.now();
      let consumersChanged = false;
      for (const [id, c] of consumersRef.current) {
        if (now - c.lastTs > DROP_MS) {
          consumersRef.current.delete(id);
          consumersChanged = true;
        } else {
          const active = now - c.lastTs < ACTIVE_MS;
          if (active !== c.active) {
            consumersRef.current.set(id, { ...c, active });
            consumersChanged = true;
          }
        }
      }
      if (consumersChanged) setConsumers([...consumersRef.current.values()]);

      const liveBeams = beamsRef.current.filter((b) => now - b.startTs < b.ttl + 100);
      if (liveBeams.length !== beamsRef.current.length) {
        beamsRef.current = liveBeams;
        setBeams(liveBeams);
      }

      const cutoff = now - 60_000;
      eventTimesRef.current = eventTimesRef.current.filter((x) => x >= cutoff);
      setEventsPerMin(eventTimesRef.current.length);
    }, SWEEP_MS);
    return () => clearInterval(t);
  }, []);

  // The socket. Torn down (and not reopened) whenever `enabled` is false — this
  // is the OFF path that saves resources when the "Live" toggle is unchecked.
  useEffect(() => {
    if (!enabled || !endpoint || !token) {
      setStatus("idle");
      return;
    }
    const session = new ConsumerLiveSession({
      endpoint,
      token,
      onActivity: ingest,
      onStatus: (s) => setStatus(s === "live" ? "live" : "idle"),
    });
    return () => session.close();
  }, [enabled, endpoint, token, ingest]);

  return useMemo(
    () => ({ enabled, consumers, beams, status, eventsPerMin }),
    [enabled, consumers, beams, status, eventsPerMin],
  );
}
