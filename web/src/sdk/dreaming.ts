// Typed client for the kyma "dreaming" layer — the agentic memory consolidation
// runs (`/v1/agent/memory/dreaming/*`), the worker registry (`/v1/workers`), and
// the agent-run trace drilldown (`/v1/agent/runs/:id`). JSON in/out.
//
// Mirrors the per-module fetch-helper convention of `sdk/connectors.ts`:
// `base()` trims the trailing slash, `headers()` carries the bearer + x-database,
// `handleResponse` / `handleEmpty` normalize the error surface.

// ── Run types ─────────────────────────────────────────────────────────────────

export type RunKind = "dreaming" | "consolidation";
export type RunStatus = "running" | "success" | "error" | "skipped";
export type RunTrigger = "scheduled" | "manual";

/** Aggregate outcome counters for a finished (or in-progress) run. */
export interface RunStats {
  memories_created: number;
  memories_merged: number;
  memories_archived: number;
  importance_rescored: number;
  judgements: number;
  entities_linked: number;
  connector_reads: number;
  tool_calls: number;
  summary: string;
}

/** A single activity-feed entry in a run's rolling ring (newest last). */
export interface ProgressActivity {
  icon: string;
  text: string;
  ts: string;
}

/** Live progress snapshot — populated for running runs, final for completed. */
export interface RunProgress {
  current_phase: string;
  activity: ProgressActivity[];
  thinking: string | null;
  counters: Record<string, unknown>;
}

export interface Run {
  id: string;
  kind: RunKind;
  status: RunStatus;
  mode: string;
  trigger: RunTrigger;
  engine: string | null;
  model: string | null;
  worker_id: string | null;
  started_at: string;
  finished_at: string | null;
  events_scanned: number;
  memories_written: number;
  error: string | null;
  job_id: string | null;
  session_id: string | null;
  agent_run_id: string | null;
  stats: RunStats | null;
  progress: RunProgress | null;
}

interface RunsEnvelope {
  items: Run[];
}

export interface TriggerResult {
  /** null when a run was already in flight (deduped). */
  job_id: string | null;
  deduped?: boolean;
  detail?: string;
}

// ── Worker types (GET /v1/workers) ──────────────────────────────────────────────

export type WorkerKind = "embedded" | "daemon";
export type WorkerStatus = "online" | "draining" | "offline";

export interface WorkerPresence {
  session_id: string;
  realm: string;
  last_activity: string;
}

export interface Worker {
  id: string;
  name: string;
  hostname: string;
  kind: WorkerKind;
  capabilities: string[];
  labels: Record<string, unknown>;
  sources: Record<string, unknown>;
  status: WorkerStatus;
  max_concurrent: number;
  version: string;
  presence: WorkerPresence[];
  last_heartbeat: string | null;
  created_at: string;
}

interface WorkersEnvelope {
  items: Worker[];
}

// ── Agent-run trace (GET /v1/agent/runs/:id) ────────────────────────────────────

/** One frame of the recorded agent trace. `data` shape varies by `event`. */
export interface TraceFrame {
  event:
    | "session"
    | "run_started"
    | "answer_delta"
    | "thinking_delta"
    | "tool_call"
    | "tool_result"
    | "run_error"
    | "answer_final"
    | string;
  data: Record<string, unknown>;
}

export interface AgentRunTrace {
  run_id: string;
  question: string;
  model_id: string | null;
  status: string;
  started_at: string;
  finished_at: string | null;
  usage: Record<string, unknown> | null;
  trace: TraceFrame[];
}

// ── Helpers ─────────────────────────────────────────────────────────────────────

type BaseArgs = { endpoint: string; token: string; database?: string };

function headers(token: string, database?: string): Record<string, string> {
  const h: Record<string, string> = {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
  if (database) h["x-database"] = database;
  return h;
}

function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

// ── API functions ─────────────────────────────────────────────────────────────

export async function listDreamingRuns(
  args: BaseArgs & { kind?: RunKind; limit?: number; offset?: number },
): Promise<Run[]> {
  const q = new URLSearchParams();
  if (args.kind) q.set("kind", args.kind);
  if (args.limit != null) q.set("limit", String(args.limit));
  if (args.offset != null) q.set("offset", String(args.offset));
  const qs = q.toString();
  const res = await fetch(
    `${base(args.endpoint)}/v1/agent/memory/dreaming/runs${qs ? `?${qs}` : ""}`,
    { headers: headers(args.token, args.database) },
  );
  const body = await handleResponse<RunsEnvelope>(res);
  return body.items ?? [];
}

export async function getDreamingRun(
  args: BaseArgs & { id: string },
): Promise<Run> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/agent/memory/dreaming/runs/${args.id}`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<Run>(res);
}

export async function triggerDreamingRun(
  args: BaseArgs & { mode?: string; focus?: string },
): Promise<TriggerResult> {
  const body: Record<string, string> = {};
  if (args.mode) body.mode = args.mode;
  if (args.focus) body.focus = args.focus;
  const res = await fetch(`${base(args.endpoint)}/v1/agent/memory/dreaming/run`, {
    method: "POST",
    headers: headers(args.token, args.database),
    body: JSON.stringify(body),
  });
  // 202 (queued) and 200 (deduped) both carry a JSON body.
  return handleResponse<TriggerResult>(res);
}

export async function listWorkers(args: BaseArgs): Promise<Worker[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/workers`, {
    headers: headers(args.token, args.database),
  });
  const body = await handleResponse<WorkersEnvelope>(res);
  return body.items ?? [];
}

export async function getAgentRunTrace(
  args: BaseArgs & { agentRunId: string },
): Promise<AgentRunTrace> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/agent/runs/${args.agentRunId}`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<AgentRunTrace>(res);
}
