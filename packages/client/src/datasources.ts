// Typed client for the kyma data sources layer (`/v1/data-sources/*`). JSON in/out.
// Adds `handleEmpty` for the 204/202 mutation responses.

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

// ── Types ─────────────────────────────────────────────────────────────────────

/** List item — the wrapped `{ items: [...] }` shape carries no metrics. */
export interface DataSourceSummary {
  id: string;
  name: string;
  type: string;
  enabled: boolean;
}

/** Full detail — `config` arrives secret-scrubbed (`***`). */
export interface DataSourceDetail {
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
  /** Stored credential this data source authenticates with, if any. */
  credential_id?: string | null;
}

export interface CreateDataSourceBody {
  name: string;
  type: string;
  target_database: string;
  target_table: string;
  schedule_ms: number;
  config: Record<string, unknown>;
  /** Stored credential id (OAuth/PAT) the data source resolves at run time. */
  credential_id?: string | null;
}

export interface DataSourceUpdate {
  name?: string;
  schedule_ms?: number;
  enabled?: boolean;
  config?: Record<string, unknown>;
  credential_id?: string | null;
}

// ── Catalog (engine-driven; GET /v1/data-sources/catalog) ───────────────────────

/** A config input the wizard renders for a data source kind. */
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

/** Self-describing data source metadata, served by the engine. */
export interface CatalogEntry {
  type_id: string;
  label: string;
  category: "code" | "knowledge" | "project" | "data" | string;
  description: string;
  /** simple-icons slug for the brand mark (e.g. "github") */
  brand: string;
  auth_mode: "pat" | "oauth" | "url" | "none" | string;
  status: "available" | "installed" | "coming_soon";
  /**
   * "periodic" — ticked by the scheduler on schedule_ms; "continuous" —
   * synced by a node-local file watcher (no interval to configure).
   */
  drive_model: "periodic" | "continuous" | string;
  default_schedule_ms: number;
  fields: CatalogField[];
  resource?: CatalogResource;
  default_target_table?: string;
  config_defaults?: Record<string, unknown>;
  graph_name?: string;
  /** OAuth data sources: provider slug to start the connect flow with. */
  oauth_provider?: string;
  /** OAuth data sources: default scopes the UI requests. */
  oauth_scopes?: string[];
  /**
   * Credential kinds this data source accepts via `credential_id` in its config
   * (e.g. ["service_principal"]). The wizard renders a stored-credential
   * picker filtered to these kinds; absent/empty means inline secrets only.
   */
  accepted_credential_kinds?: string[];
}

interface CatalogEnvelope {
  items: CatalogEntry[];
}

export async function getDataSourceCatalog(t: KymaTransport): Promise<CatalogEntry[]> {
  const body = await handleResponse<CatalogEnvelope>(await t.request("/v1/data-sources/catalog"));
  return body.items ?? [];
}

/** A repository as returned by `POST /v1/data-sources/github/repos`. */
export interface GitHubRepo {
  full_name: string;
  name: string;
  owner: string;
  private: boolean;
  default_branch: string;
  description: string | null;
}

/** Derived (client-side) data source status — there is no server enum. */
export type DataSourceStatus =
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
  items: DataSourceSummary[];
}

export async function listDataSources(t: KymaTransport): Promise<DataSourceSummary[]> {
  const body = await handleResponse<ListEnvelope>(await t.request("/v1/data-sources"));
  return body.items ?? [];
}

export async function getDataSource(
  t: KymaTransport,
  args: { id: string },
): Promise<DataSourceDetail> {
  return handleResponse<DataSourceDetail>(await t.request(`/v1/data-sources/${args.id}`));
}

export async function createDataSource(
  t: KymaTransport,
  args: { body: CreateDataSourceBody },
): Promise<{ id: string }> {
  return handleResponse<{ id: string }>(
    await t.request("/v1/data-sources", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.body),
    }),
  );
}

export async function patchDataSource(
  t: KymaTransport,
  args: { id: string; patch: DataSourceUpdate },
): Promise<void> {
  return handleEmpty(
    await t.request(`/v1/data-sources/${args.id}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.patch),
    }),
  );
}

export async function deleteDataSource(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/data-sources/${args.id}`, { method: "DELETE" }));
}

export async function pauseDataSource(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/data-sources/${args.id}/pause`, { method: "POST" }));
}

export async function resumeDataSource(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/data-sources/${args.id}/resume`, { method: "POST" }));
}

export async function triggerDataSource(t: KymaTransport, args: { id: string }): Promise<void> {
  return handleEmpty(await t.request(`/v1/data-sources/${args.id}/trigger`, { method: "POST" }));
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
    await t.request("/v1/data-sources/github/repos", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ token: args.pat }),
    }),
  );
  return body.repos ?? [];
}

// ── Derived status ──────────────────────────────────────────────────────────────

/**
 * Synthesize a data source status from the detail fields — the server exposes no
 * status enum. Precedence: disabled → error → syncing (a run started but no
 * success yet) → synced (at least one success) → idle (never run).
 */
export function deriveStatus(detail: DataSourceDetail): DataSourceStatus {
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

// ── Watchers (file watchers / cc-sync provenance) ─────────────────────────────

export interface DataSourceWatcher {
  id: string;
  kind: "filedrop" | "cc_sync" | "obsidian" | string;
  node_host: string;
  node_id: string;
  identity: string;
  config: Record<string, unknown>;
  started_at: string;
  last_heartbeat_at: string;
  last_scan: {
    seen: number;
    processed: number;
    errors: number;
    duration_ms: number;
    at: string;
    detail?: Record<string, unknown>;
  } | null;
  stale: boolean;
}

export async function listDataSourceWatchers(t: KymaTransport): Promise<DataSourceWatcher[]> {
  const body = await handleResponse<{ items: DataSourceWatcher[] }>(
    await t.request("/v1/data-sources/watchers"),
  );
  return body.items ?? [];
}

// ── Watcher settings (local-mode toggle for cc-sync) ──────────────────────────

export interface WatcherSettings {
  cc_sync_enabled: boolean;
}

export async function getWatcherSettings(t: KymaTransport): Promise<WatcherSettings> {
  return handleResponse<WatcherSettings>(
    await t.request("/v1/data-sources/watchers/settings"),
  );
}

export async function updateWatcherSettings(
  t: KymaTransport,
  patch: Partial<WatcherSettings>,
): Promise<WatcherSettings> {
  return handleResponse<WatcherSettings>(
    await t.request("/v1/data-sources/watchers/settings", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(patch),
    }),
  );
}
