//! Typed client for `GET /v1/agent/memory/overview` — powers the Agent-tab
//! Memory ingestion panel (firehose activity, memory store, pipeline runs).

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

export type Row = Record<string, unknown>;

export interface PipelineRun {
  id: string;
  kind: string;
  status: "running" | "success" | "error" | "skipped" | string;
  started_at: string;
  finished_at: string | null;
  events_scanned: number;
  memories_written: number;
  error: string | null;
}

export interface MemoryOverview {
  memory: { counts: Row[]; recent: Row[] };
  firehose: {
    by_kind: Row[];
    timeline: Row[];
    sessions: Row[];
    recent: Row[];
  };
  pipeline_runs: PipelineRun[];
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

export async function fetchMemoryOverview(t: KymaTransport): Promise<MemoryOverview> {
  return handleResponse<MemoryOverview>(await t.request("/v1/agent/memory/overview"));
}

// ── graph-aware hybrid recall (POST /v1/agent/memory/query) ──────────────────

export interface RetrievedMemory {
  id: string;
  memory_type: string;
  title: string | null;
  content_preview: string;
  score: number;
  distance: number | null;
  kw_score: number | null;
  graph_proximity: number;
  importance: number;
  realm: string;
  valid_at: string | null;
  invalid_at: string | null;
  via: { seed?: string; type?: string; depth?: number } | null;
}

export interface LinkedResource {
  node_id: string;
  target_namespace: string | null;
  edge_type: string;
  depth: number;
}

export interface MemoryQueryResult {
  memories: RetrievedMemory[];
  linked: LinkedResource[];
  context: string;
  brief?: string;
  took_ms: number;
}

export interface MemoryQueryRequest {
  query: string;
  mode?: "fast" | "agentic";
  limit?: number;
  realms?: string[];
  memory_type?: string;
  tags?: string[];
  importance_min?: number;
  as_of?: string;
  include_invalidated?: boolean;
  expand_hops?: number;
}

export async function queryMemory(
  t: KymaTransport,
  req: MemoryQueryRequest,
): Promise<MemoryQueryResult> {
  return handleResponse<MemoryQueryResult>(
    await t.request("/v1/agent/memory/query", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(req),
    }),
  );
}

// ── tunable settings (GET/PUT /v1/agent/memory/settings) ─────────────────────

/** The autonomous "dreaming" consolidation block of the memory settings. */
export interface DreamingSettings {
  enabled: boolean;
  interval_secs: number;
  mode: "full" | "housekeeping_only" | "sources";
  realm_scope: string[];
  max_tool_calls: number;
  wall_clock_secs: number;
  data_source_read_budget: number;
  data_source_read_max_bytes: number;
  mutation_cap: number;
}

/** A class of memory mutation the HITL policy can govern. */
export type MemoryOp =
  | "add"
  | "update"
  | "invalidate"
  | "merge"
  | "archive"
  | "link_entity_cross_realm"
  | "relationship_write"
  | "promote_file_candidate";

/** Per-op base behavior: apply silently / apply-and-flag / hold for approval. */
export type OpMode = "auto" | "post_hoc" | "gate";

/** Operator-configurable human-in-the-loop policy over automatic memory mutations. */
export interface HitlPolicy {
  /** Master switch. When false, all mutations apply directly (default). */
  enabled: boolean;
  /** Base mode per op class. */
  ops: Partial<Record<MemoryOp, OpMode>>;
  /** Confidence below this escalates an op one severity level. */
  confidence_threshold: number;
  /** Realms the policy applies to (empty = all). */
  realm_scope: string[];
  /** Memory types the policy applies to (empty = all). */
  type_scope: string[];
}

export interface MemorySettings {
  // ingestion
  extraction_enabled: boolean;
  min_events: number;
  // retrieval / ranking
  default_limit: number;
  default_expand_hops: number;
  ann_threshold: number;
  w_rrf: number;
  w_semantic: number;
  w_keyword: number;
  w_graph: number;
  w_importance: number;
  w_recency: number;
  half_life_days: number;
  rrf_k: number;
  // dreaming (scheduled agentic housekeeping)
  dreaming: DreamingSettings;
  // human-in-the-loop approval policy (default off)
  hitl: HitlPolicy;
}

export async function getMemorySettings(t: KymaTransport): Promise<MemorySettings> {
  return handleResponse<MemorySettings>(await t.request("/v1/agent/memory/settings"));
}

export async function putMemorySettings(
  t: KymaTransport,
  s: MemorySettings,
): Promise<void> {
  const res = await t.request("/v1/agent/memory/settings", {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(s),
  });
  if (!res.ok) throw await errorFromResponse(res);
}

// ── provenance summary (GET /v1/agent/memory/source-summary) ─────────────────

/** Count of non-archived memories formed by `source` (e.g. "claude-code",
 * "dreaming"; no provenance source = "manual") in `realm`. Powers the Data
 * Sources "Memories" tab. */
export interface MemorySourceSummary {
  source: string;
  realm: string;
  count: number;
}

export async function memorySourceSummary(
  t: KymaTransport,
): Promise<MemorySourceSummary[]> {
  const body = await handleResponse<{ items: MemorySourceSummary[] }>(
    await t.request("/v1/agent/memory/source-summary"),
  );
  return body.items ?? [];
}
