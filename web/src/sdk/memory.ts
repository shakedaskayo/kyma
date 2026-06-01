//! Typed client for `GET /v1/agent/memory/overview` — powers the Agent-tab
//! Memory ingestion panel (firehose activity, memory store, pipeline runs).

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

type Args = { endpoint: string; token: string };

function base(endpoint: string) {
  return endpoint.replace(/\/$/, "");
}

export async function fetchMemoryOverview(a: Args): Promise<MemoryOverview> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/memory/overview`, {
    headers: {
      authorization: `Bearer ${a.token}`,
      "content-type": "application/json",
    },
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`memory overview: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
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
  a: Args,
  req: MemoryQueryRequest,
): Promise<MemoryQueryResult> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/memory/query`, {
    method: "POST",
    headers: {
      authorization: `Bearer ${a.token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(req),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`memory query: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}

// ── tunable settings (GET/PUT /v1/agent/memory/settings) ─────────────────────

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
}

export async function getMemorySettings(a: Args): Promise<MemorySettings> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/memory/settings`, {
    headers: {
      authorization: `Bearer ${a.token}`,
      "content-type": "application/json",
    },
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`memory settings: ${res.status}${t ? ` — ${t}` : ""}`);
  }
  return res.json();
}

export async function putMemorySettings(
  a: Args,
  s: MemorySettings,
): Promise<void> {
  const res = await fetch(`${base(a.endpoint)}/v1/agent/memory/settings`, {
    method: "PUT",
    headers: {
      authorization: `Bearer ${a.token}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(s),
  });
  if (!res.ok) {
    const t = await res.text().catch(() => "");
    throw new Error(`memory settings: ${res.status}${t ? ` — ${t}` : ""}`);
  }
}
