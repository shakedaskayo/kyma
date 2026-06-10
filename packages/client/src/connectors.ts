// Typed client for the kyma connectors layer (`/v1/connectors/*`). JSON in/out.
// Adds `handleEmpty` for the 204/202 mutation responses.

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

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
  /** Stored credential this connector authenticates with, if any. */
  credential_id?: string | null;
}

export interface CreateConnectorBody {
  name: string;
  type: string;
  target_database: string;
  target_table: string;
  schedule_ms: number;
  config: Record<string, unknown>;
  /** Stored credential id (OAuth/PAT) the connector resolves at run time. */
  credential_id?: string | null;
}

export interface ConnectorUpdate {
  name?: string;
  schedule_ms?: number;
  enabled?: boolean;
  config?: Record<string, unknown>;
  credential_id?: string | null;
}

// ── Catalog (engine-driven; GET /v1/connectors/catalog) ───────────────────────

/** A config input the wizard renders for a connector kind. */
export interface CatalogField {
  key: string;
  label: string;
  type: "text" | "secret" | "checkbox";
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
  /** OAuth connectors: provider slug to start the connect flow with. */
  oauth_provider?: string;
  /** OAuth connectors: default scopes the UI requests. */
  oauth_scopes?: string[];
  /**
   * Credential kinds this connector accepts via `credential_id` in its config
   * (e.g. ["service_principal"]). The wizard renders a stored-credential
   * picker filtered to these kinds; absent/empty means inline secrets only.
   */
  accepted_credential_kinds?: string[];
}

interface CatalogEnvelope {
  items: CatalogEntry[];
}

export async function getConnectorCatalog(t: KymaTransport): Promise<CatalogEntry[]> {
  const body = await handleResponse<CatalogEnvelope>(await t.request("/v1/connectors/catalog"));
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

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

/** For 204 (DELETE/PATCH/pause/resume) and 202 (trigger) — no body to parse. */
async function handleEmpty(res: Response): Promise<void> {
  if (!res.ok) throw await errorFromResponse(res);
}

// ── API functions ─────────────────────────────────────────────────────────────

interface ListEnvelope {
  items: ConnectorSummary[];
}

export async function listConnectors(t: KymaTransport): Promise<ConnectorSummary[]> {
  const body = await handleResponse<ListEnvelope>(await t.request("/v1/connectors"));
  return body.items ?? [];
}

export async function getConnector(
  t: KymaTransport,
  args: { id: string },
): Promise<ConnectorDetail> {
  return handleResponse<ConnectorDetail>(await t.request(`/v1/connectors/${args.id}`));
}

export async function createConnector(
  t: KymaTransport,
  args: { body: CreateConnectorBody },
): Promise<{ id: string }> {
  return handleResponse<{ id: string }>(
    await t.request("/v1/connectors", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.body),
    }),
  );
}

export async function patchConnector(
  t: KymaTransport,
  args: { id: string; patch: ConnectorUpdate },
): Promise<void> {
  return handleEmpty(
    await t.request(`/v1/connectors/${args.id}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.patch),
    }),
  );
}

export async function deleteConnector(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/connectors/${args.id}`, { method: "DELETE" }));
}

export async function pauseConnector(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/connectors/${args.id}/pause`, { method: "POST" }));
}

export async function resumeConnector(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/connectors/${args.id}/resume`, { method: "POST" }));
}

export async function triggerConnector(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/connectors/${args.id}/trigger`, { method: "POST" }));
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
  t: KymaTransport,
  args: { pat: string },
): Promise<GitHubRepo[]> {
  const body = await handleResponse<ReposEnvelope>(
    await t.request("/v1/connectors/github/repos", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token: args.pat }),
    }),
  );
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
  // A run is in flight when it started but hasn't recorded a success yet, or
  // re-ran after the last success (last_run_at strictly newer than last_success_at).
  if (
    detail.last_run_at &&
    (!detail.last_success_at ||
      new Date(detail.last_run_at) > new Date(detail.last_success_at))
  ) {
    return "syncing";
  }
  if (detail.last_success_at) return "synced";
  return "idle";
}
