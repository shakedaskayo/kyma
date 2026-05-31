// Typed client for the kyma connectors layer (`/v1/connectors/*`). JSON in/out.
// Mirrors the per-module fetch-helper convention used by `sdk/graph.ts` /
// `sdk/dashboards.ts`. Adds `handleEmpty` for the 204/202 mutation responses.

// ── Types ─────────────────────────────────────────────────────────────────────

/** List item — the wrapped `{ items: [...] }` shape carries no metrics. */
export interface ConnectorSummary {
  id: string;
  name: string;
  type: string;
  enabled: boolean;
}

/** Full detail — `config` arrives secret-scrubbed (`***`). */
export interface ConnectorDetail {
  id: string;
  name: string;
  type: string;
  target_database: string;
  target_table: string;
  schedule_ms: number;
  drive_model: string;
  enabled: boolean;
  disabled_reason: string | null;
  last_run_at: string | null;
  last_success_at: string | null;
  last_error: string | null;
  last_rows_ingested: number | null;
  config: Record<string, unknown>;
}

export interface CreateConnectorBody {
  name: string;
  type: string;
  target_database: string;
  target_table: string;
  schedule_ms: number;
  config: Record<string, unknown>;
}

export interface ConnectorUpdate {
  name?: string;
  schedule_ms?: number;
  enabled?: boolean;
  config?: Record<string, unknown>;
}

// ── Catalog (engine-driven; GET /v1/connectors/catalog) ───────────────────────

/** A config input the wizard renders for a connector kind. */
export interface CatalogField {
  key: string;
  label: string;
  type: "text" | "secret";
  required: boolean;
  placeholder?: string;
  help?: string;
}

/** Optional resource-selection step (e.g. picking repositories). */
export interface CatalogResource {
  label: string;
  /** config key the selected resources are written under (e.g. "repos") */
  config_key: string;
  /** the secret field whose value unlocks fetching the list (e.g. "token") */
  token_field: string;
}

/** Self-describing connector metadata, served by the engine. */
export interface CatalogEntry {
  type_id: string;
  label: string;
  category: "code" | "knowledge" | "project" | "data" | string;
  description: string;
  /** simple-icons slug for the brand mark (e.g. "github") */
  brand: string;
  auth_mode: "pat" | "oauth" | "url" | "none" | string;
  status: "available" | "coming_soon";
  default_schedule_ms: number;
  fields: CatalogField[];
  resource?: CatalogResource;
  default_target_table?: string;
  config_defaults?: Record<string, unknown>;
  graph_name?: string;
}

interface CatalogEnvelope {
  items: CatalogEntry[];
}

export async function getConnectorCatalog(args: BaseArgs): Promise<CatalogEntry[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/catalog`, {
    headers: headers(args.token, args.database),
  });
  const body = await handleResponse<CatalogEnvelope>(res);
  return body.items ?? [];
}

/** A repository as returned by `POST /v1/connectors/github/repos`. */
export interface GitHubRepo {
  full_name: string;
  name: string;
  owner: string;
  private: boolean;
  default_branch: string;
  description: string | null;
}

/** Derived (client-side) connector status — there is no server enum. */
export type ConnectorStatus =
  | "disabled"
  | "error"
  | "syncing"
  | "synced"
  | "idle";

/** schedule_ms is clamped server-side to this inclusive range. */
export const SCHEDULE_MS_MIN = 100;
export const SCHEDULE_MS_MAX = 86_400_000;

// ── Helpers ───────────────────────────────────────────────────────────────────

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

/** For 204 (DELETE/PATCH/pause/resume) and 202 (trigger) — no body to parse. */
async function handleEmpty(res: Response): Promise<void> {
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
}

// ── API functions ─────────────────────────────────────────────────────────────

interface ListEnvelope {
  items: ConnectorSummary[];
}

export async function listConnectors(args: BaseArgs): Promise<ConnectorSummary[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors`, {
    headers: headers(args.token, args.database),
  });
  const body = await handleResponse<ListEnvelope>(res);
  return body.items ?? [];
}

export async function getConnector(
  args: BaseArgs & { id: string },
): Promise<ConnectorDetail> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/${args.id}`, {
    headers: headers(args.token, args.database),
  });
  return handleResponse<ConnectorDetail>(res);
}

export async function createConnector(
  args: BaseArgs & { body: CreateConnectorBody },
): Promise<{ id: string }> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors`, {
    method: "POST",
    headers: headers(args.token, args.database),
    body: JSON.stringify(args.body),
  });
  return handleResponse<{ id: string }>(res);
}

export async function patchConnector(
  args: BaseArgs & { id: string; patch: ConnectorUpdate },
): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/${args.id}`, {
    method: "PATCH",
    headers: headers(args.token, args.database),
    body: JSON.stringify(args.patch),
  });
  return handleEmpty(res);
}

export async function deleteConnector(args: BaseArgs & { id: string }): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/${args.id}`, {
    method: "DELETE",
    headers: headers(args.token, args.database),
  });
  return handleEmpty(res);
}

export async function pauseConnector(args: BaseArgs & { id: string }): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/${args.id}/pause`, {
    method: "POST",
    headers: headers(args.token, args.database),
  });
  return handleEmpty(res);
}

export async function resumeConnector(args: BaseArgs & { id: string }): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/${args.id}/resume`, {
    method: "POST",
    headers: headers(args.token, args.database),
  });
  return handleEmpty(res);
}

export async function triggerConnector(args: BaseArgs & { id: string }): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/${args.id}/trigger`, {
    method: "POST",
    headers: headers(args.token, args.database),
  });
  return handleEmpty(res);
}

interface ReposEnvelope {
  repos: GitHubRepo[];
}

/**
 * Fetch the repositories visible to a GitHub PAT for the wizard's repo picker.
 * Behind `Role::Write`, so the session bearer goes in the header while the PAT
 * travels in the request body (never a query string, never a cache key).
 */
export async function listGitHubRepos(
  args: BaseArgs & { pat: string },
): Promise<GitHubRepo[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/connectors/github/repos`, {
    method: "POST",
    headers: headers(args.token, args.database),
    body: JSON.stringify({ token: args.pat }),
  });
  const body = await handleResponse<ReposEnvelope>(res);
  return body.repos ?? [];
}

// ── Derived status ──────────────────────────────────────────────────────────────

/**
 * Synthesize a connector status from the detail fields — the server exposes no
 * status enum. Precedence: disabled → error → syncing (a run started but no
 * success yet) → synced (at least one success) → idle (never run).
 */
export function deriveStatus(detail: ConnectorDetail): ConnectorStatus {
  if (!detail.enabled) return "disabled";
  if (detail.last_error) return "error";
  if (detail.last_run_at && detail.last_run_at !== detail.last_success_at && !detail.last_success_at) {
    return "syncing";
  }
  if (detail.last_success_at) return "synced";
  if (detail.last_run_at) return "syncing";
  return "idle";
}
