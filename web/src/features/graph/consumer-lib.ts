/**
 * Pure helpers + tuning constants for the live-consumers stream. Maps raw
 * backend activity frames into the SDK's `LiveConsumer` / `ConsumerBeam` shapes.
 */
import { resolveConsumerKind } from "@kyma-ai/react/graph";
import type { ConsumerBeam, ConsumerVerb, LiveConsumer } from "@kyma-ai/react/graph";
import type { ConsumerActivityFrame } from "./consumer-live";

/** A consumer is "active" (bright, beam-eligible) within this window. */
export const ACTIVE_MS = 10_000;
/** ...and stays listed (faded) until this long after its last activity. */
export const DROP_MS = 120_000;
/** Beam lifetime — within the 1–2s window from the design. */
export const BEAM_TTL = 1_600;
/** Hard cap on concurrent beams to keep the overlay cheap. */
export const MAX_BEAMS = 24;
/** How often the hook sweeps idle consumers + prunes expired beams. */
export const SWEEP_MS = 1_000;

export function consumerVerb(action: string): ConsumerVerb {
  if (action === "remember") return "remember";
  if (action === "search") return "search";
  return "recall";
}

/** Stable per-event key for dedup across reconnect/backfill overlap. */
export function dedupKey(a: ConsumerActivityFrame): string {
  return `${a.ts}|${a.consumer_id}|${a.action}|${a.node_ids.join(",")}`;
}

/** Build/refresh a `LiveConsumer` from an activity frame. */
export function toConsumer(a: ConsumerActivityFrame, active: boolean): LiveConsumer {
  return {
    id: a.consumer_id,
    kind: resolveConsumerKind(a.kind),
    source: a.kind,
    label: a.label || a.kind,
    lastTs: a.ts,
    lastVerb: consumerVerb(a.action),
    lastTargetCount: a.node_ids.length,
    lastNamespaces: a.namespaces,
    active,
  };
}

/** One beam per touched node (raw ids; the SDK resolves them to graph nodes). */
export function toBeams(a: ConsumerActivityFrame, startTs: number): ConsumerBeam[] {
  const verb = consumerVerb(a.action);
  return a.node_ids.map((rawNodeId, i) => ({
    id: `${a.consumer_id}|${rawNodeId}|${a.ts}|${i}`,
    consumerId: a.consumer_id,
    rawNodeId,
    verb,
    startTs,
    ttl: BEAM_TTL,
  }));
}
