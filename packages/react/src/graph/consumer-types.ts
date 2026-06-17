/**
 * Live-consumers overlay — shared data contract.
 *
 * These shapes are the boundary between the host app (which owns the WebSocket
 * that streams consumer activity) and this SDK (which renders it). The host
 * passes a `LiveConsumersData` snapshot into `<KymaGraph liveConsumers={…} />`;
 * the SDK never opens a socket. Kept in the SDK so the web hook imports one
 * source of truth (no web → SDK type drift).
 */

/** Normalized agent kind for icon/colour mapping. Agent-agnostic — unknown
 *  kinds fall back to a neutral default. */
export type ConsumerKind =
  | "claude_code"
  | "cursor"
  | "windsurf"
  | "kyma_agent"
  | "mcp"
  | "unknown";

/** What a consumer did to memory. */
export type ConsumerVerb = "recall" | "search" | "remember";

/** Connection state of the live feed, mirroring the memory firehose. */
export type ConsumerStatus = "live" | "polling" | "idle";

/** An agent currently (or recently) reading/writing memory. */
export interface LiveConsumer {
  /** Stable grouping key from the backend (`{kind}:{subject|anon}`). */
  id: string;
  kind: ConsumerKind;
  /** Raw backend source string (e.g. `claude-code`, `mcp-stdio`). */
  source: string;
  /** Human display label. */
  label: string;
  /** Millis of the most recent activity. */
  lastTs: number;
  lastVerb: ConsumerVerb | null;
  /** How many nodes the last activity touched. */
  lastTargetCount: number;
  /** Realms/namespaces the last activity touched (for the dock subtitle). */
  lastNamespaces: string[];
  /** False once past the idle threshold — the dock fades it before removal. */
  active: boolean;
  /** Last query text (recall/search), for the expanded detail row. */
  lastQuery: string | null;

  // ── connection / process detail (best-effort; shown in the expanded row) ──
  /** Host machine (the server; = the client machine in local mode). */
  host: string | null;
  /** MCP client version, when advertised. */
  clientVersion: string | null;
  /** Transport: `local-serve` | `server` | `mcp-stdio`. */
  transport: string | null;
  /** Peer IP (loopback locally, the agent's IP in server mode). */
  ip: string | null;
  /** Best-effort local process id (loopback connections only). */
  pid: number | null;
  /** Auth subject, when present. */
  subject: string | null;
  /** Tenant id. */
  tenant: string | null;

  // ── session accumulation (tracked by the host hook) ──
  /** Total operations observed for this consumer this session. */
  opsCount: number;
  /** Millis when this consumer was first seen. */
  firstTs: number;
}

/** A single in-flight beam from a consumer to a node it touched. The node id is
 *  raw (`memory:<uuid>` / a resource id); the SDK resolves it against the loaded
 *  graph each frame and skips drawing when the node isn't on the canvas. */
export interface ConsumerBeam {
  id: string;
  consumerId: string;
  rawNodeId: string;
  verb: ConsumerVerb;
  /** `Date.now()` at arrival. */
  startTs: number;
  /** Lifetime in ms; the beam decays to nothing over this window. */
  ttl: number;
}

/** The full snapshot the host passes into `KymaGraph`. */
export interface LiveConsumersData {
  /** Whether the host's stream is active (mirrors the in-graph toggle). */
  enabled: boolean;
  consumers: LiveConsumer[];
  beams: ConsumerBeam[];
  status: ConsumerStatus;
  eventsPerMin: number;
}
