//! Typed client for brain repos (`/v1/brain/*`) — pensieve memory published as
//! Git-clonable Obsidian vaults served over smart HTTP at `/git/<name>.git`.

import type { PensieveTransport } from "./transport";
import { errorFromResponse } from "./errors";

export type RealmSelector =
  | { kind: "all"; realms?: undefined }
  | { kind: "realms"; realms: string[] };

export interface BrainFilters {
  memory_types?: string[] | null;
  statuses: string[];
  min_importance?: number | null;
  redact_private_spans: boolean;
  include_invalidated: boolean;
}

export interface GardenerConfig {
  enabled: boolean;
  interval_secs: number;
}

export interface BrainConfig {
  name: string;
  realms: RealmSelector;
  layout: "flat" | "by_realm";
  include: BrainFilters;
  visibility_role: string;
  export_interval_secs: number;
  gardener: GardenerConfig;
  created_at: string;
  updated_at: string;
}

export interface BrainRunRecord {
  kind: "export" | "push_ingest" | "gardener";
  started_at: string;
  finished_at: string;
  commit?: string | null;
  files_written: number;
  files_deleted: number;
  notes_ingested: number;
  noop: boolean;
  error?: string | null;
  warnings?: string[];
}

export interface BrainRuntime {
  last_export_at?: string | null;
  last_commit?: string | null;
  last_gardener_at?: string | null;
  last_error?: string | null;
  note_count: number;
  runs: BrainRunRecord[];
}

export interface Brain {
  config: BrainConfig;
  runtime: BrainRuntime;
  clone_path: string;
  git_available: boolean;
}

export interface BrainsEnvelope {
  git_available: boolean;
  brains: Brain[];
}

export interface CreateBrainInput {
  name: string;
  realms?: string[];
  all_realms?: boolean;
  export_interval_secs?: number;
  gardener?: { enabled: boolean };
}

export interface BrainExportResult {
  commit?: string;
  noop?: boolean;
  notes?: number;
  files?: number;
  error?: string;
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

export async function listBrains(t: PensieveTransport): Promise<BrainsEnvelope> {
  return handleResponse(await t.request("/v1/brain"));
}

export async function getBrain(t: PensieveTransport, name: string): Promise<Brain> {
  return handleResponse(await t.request(`/v1/brain/${encodeURIComponent(name)}`));
}

export async function createBrain(
  t: PensieveTransport,
  input: CreateBrainInput,
): Promise<{ brain: Brain; first_export: BrainExportResult }> {
  return handleResponse(
    await t.request("/v1/brain", { method: "POST", body: JSON.stringify(input) }),
  );
}

export async function deleteBrain(
  t: PensieveTransport,
  name: string,
  opts?: { purge?: boolean },
): Promise<{ deleted: string; repo_purged: boolean }> {
  return handleResponse(
    await t.request(`/v1/brain/${encodeURIComponent(name)}?purge=${opts?.purge ?? true}`, {
      method: "DELETE",
    }),
  );
}

export async function triggerBrainExport(
  t: PensieveTransport,
  name: string,
): Promise<BrainExportResult> {
  return handleResponse(
    await t.request(`/v1/brain/${encodeURIComponent(name)}/export`, {
      method: "POST",
      body: "{}",
    }),
  );
}

export async function triggerBrainGardener(
  t: PensieveTransport,
  name: string,
): Promise<{ started?: boolean; deduped?: boolean; detail?: string }> {
  return handleResponse(
    await t.request(`/v1/brain/${encodeURIComponent(name)}/garden`, {
      method: "POST",
      body: "{}",
    }),
  );
}

export async function listBrainRuns(
  t: PensieveTransport,
  name: string,
): Promise<{ runs: BrainRunRecord[] }> {
  return handleResponse(await t.request(`/v1/brain/${encodeURIComponent(name)}/runs`));
}

/** The clone URL for a brain against an endpoint. */
export function brainCloneUrl(endpoint: string, name: string): string {
  return `${endpoint.replace(/\/$/, "")}/git/${name}.git`;
}
