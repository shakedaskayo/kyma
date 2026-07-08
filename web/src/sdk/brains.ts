// Typed client for brain repos (`/v1/brain/*`) — memory published as
// Git-clonable Obsidian vaults. Mirrors the per-module fetch-helper
// convention of `sdk/dreaming.ts`.

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

export interface ExportResult {
  commit?: string;
  noop?: boolean;
  notes?: number;
  files?: number;
  error?: string;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

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
  if (!res.ok) {
    const snippet = await res
      .text()
      .then((t) => t.slice(0, 200))
      .catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

// ── API functions ────────────────────────────────────────────────────────────

export async function listBrains(args: BaseArgs): Promise<BrainsEnvelope> {
  const res = await fetch(`${base(args.endpoint)}/v1/brain`, {
    headers: headers(args.token, args.database),
  });
  return handleResponse<BrainsEnvelope>(res);
}

export async function getBrain(args: BaseArgs & { name: string }): Promise<Brain> {
  const res = await fetch(`${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}`, {
    headers: headers(args.token, args.database),
  });
  return handleResponse<Brain>(res);
}

export async function createBrain(
  args: BaseArgs & { input: CreateBrainInput },
): Promise<{ brain: Brain; first_export: ExportResult }> {
  const res = await fetch(`${base(args.endpoint)}/v1/brain`, {
    method: "POST",
    headers: headers(args.token, args.database),
    body: JSON.stringify(args.input),
  });
  return handleResponse(res);
}

export async function updateBrain(
  args: BaseArgs & {
    name: string;
    patch: Partial<Pick<BrainConfig, "export_interval_secs" | "gardener" | "visibility_role">>;
  },
): Promise<Brain> {
  const res = await fetch(`${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}`, {
    method: "PUT",
    headers: headers(args.token, args.database),
    body: JSON.stringify(args.patch),
  });
  return handleResponse<Brain>(res);
}

export async function deleteBrain(
  args: BaseArgs & { name: string; purge?: boolean },
): Promise<{ deleted: string; repo_purged: boolean }> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}?purge=${args.purge ?? true}`,
    { method: "DELETE", headers: headers(args.token, args.database) },
  );
  return handleResponse(res);
}

export async function triggerBrainExport(
  args: BaseArgs & { name: string },
): Promise<ExportResult> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}/export`,
    { method: "POST", headers: headers(args.token, args.database), body: "{}" },
  );
  return handleResponse<ExportResult>(res);
}

export async function triggerBrainGardener(
  args: BaseArgs & { name: string },
): Promise<{ started?: boolean; deduped?: boolean; detail?: string }> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}/garden`,
    { method: "POST", headers: headers(args.token, args.database), body: "{}" },
  );
  return handleResponse(res);
}

export async function listBrainRuns(
  args: BaseArgs & { name: string },
): Promise<{ runs: BrainRunRecord[] }> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}/runs`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse(res);
}

export interface BrainTree {
  head: string;
  paths: string[];
}

export interface BrainFile {
  path: string;
  head: string;
  content: string;
}

export async function getBrainTree(args: BaseArgs & { name: string }): Promise<BrainTree> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}/tree`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<BrainTree>(res);
}

export async function getBrainFile(
  args: BaseArgs & { name: string; path: string },
): Promise<BrainFile> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/brain/${encodeURIComponent(args.name)}/file?path=${encodeURIComponent(args.path)}`,
    { headers: headers(args.token, args.database) },
  );
  return handleResponse<BrainFile>(res);
}

/** The clone URL for a brain against the connected endpoint. */
export function cloneUrl(endpoint: string, name: string): string {
  return `${base(endpoint)}/git/${name}.git`;
}

export function realmsLabel(selector: RealmSelector): string {
  return selector.kind === "all" ? "all realms" : (selector.realms ?? []).join(", ");
}
