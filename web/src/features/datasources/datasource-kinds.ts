import type { CatalogEntry, CreateDataSourceBody } from "@/sdk/datasources";

// Catalog presentation + create helpers. The catalog itself is engine-driven
// (GET /v1/data-sources/catalog → CatalogEntry[]); this module only adds the
// generic create-body assembly, category ordering, and interval presets, so a
// new data source needs no change here.

/** Inputs the wizard collects before assembling the create body. */
export interface KindFormValues {
  name: string;
  scheduleMs: number;
  /** field key → value, e.g. { token: "ghp_…" } */
  fields: Record<string, string>;
  /** target db/table (advanced); fall back to entry defaults / session db. */
  targetDatabase: string;
  targetTable: string;
  /** resource selection (e.g. selected repos "owner/name") */
  resources: string[];
  /** stored credential id (OAuth data sources); set by the connect step. */
  credentialId: string | null;
}

/** Ordered category metadata for the catalog grid. */
export const CATEGORIES: { id: string; label: string }[] = [
  { id: "code", label: "Code & Repositories" },
  { id: "knowledge", label: "Knowledge & Docs" },
  { id: "project", label: "Project & Issues" },
  { id: "data", label: "Data & Observability" },
];

export function categoryLabel(id: string): string {
  return CATEGORIES.find((c) => c.id === id)?.label ?? "Other";
}

/** Short label for a data source's auth mode (shown on cards / review). */
export function authLabel(mode: string): string {
  switch (mode) {
    case "pat": return "Token auth";
    case "oauth": return "OAuth";
    case "url": return "Endpoint URL";
    case "service_principal": return "Service principal";
    case "none": return "No auth";
    default: return mode;
  }
}

export function blankValues(entry: CatalogEntry): KindFormValues {
  return {
    name: "",
    scheduleMs: entry.default_schedule_ms,
    fields: {},
    targetDatabase: "",
    targetTable: "",
    resources: [],
    credentialId: null,
  };
}

/** Whether the wizard's auth step for this entry is the OAuth connect flow. */
export function isOAuth(entry: CatalogEntry): boolean {
  return entry.auth_mode === "oauth";
}

/**
 * Continuous-drive sources sync via a node-local file watcher — there is no
 * interval to pick and no target table; the wizard hides those steps.
 */
export function isWatcherDriven(entry: CatalogEntry): boolean {
  return entry.drive_model === "continuous";
}

/** Credential kinds this entry accepts via a stored `credential_id`. */
export function acceptedCredentialKinds(entry: CatalogEntry): string[] {
  return entry.accepted_credential_kinds ?? [];
}

/**
 * Whether the wizard must collect a stored credential for this entry: it
 * accepts credential kinds (non-OAuth flow) and offers no inline secret field
 * to fall back on (e.g. msfabric — service principals are credential-only).
 * Entries with an inline secret alternative (e.g. postgres `url`) treat the
 * picker as optional.
 */
export function requiresStoredCredential(entry: CatalogEntry): boolean {
  if (isOAuth(entry) || acceptedCredentialKinds(entry).length === 0) return false;
  return !entry.fields.some((f) => f.type === "secret" && f.required);
}

/** Whether to render the stored-credential picker for this entry at all. */
export function offersStoredCredential(entry: CatalogEntry): boolean {
  return !isOAuth(entry) && acceptedCredentialKinds(entry).length > 0;
}

/**
 * Assemble the `POST /v1/data-sources` body generically from a catalog entry:
 * merge the entry's `config_defaults`, layer the collected field values, and
 * write the selected resources under the entry's `resource.config_key`.
 */
export function buildCreateBody(
  entry: CatalogEntry,
  values: KindFormValues,
  sessionDatabase: string,
): CreateDataSourceBody {
  const config: Record<string, unknown> = { ...(entry.config_defaults ?? {}) };
  for (const f of entry.fields) {
    // Checkbox fields land as JSON booleans (the wizard stores "true"/"").
    config[f.key] =
      f.type === "checkbox" ? values.fields[f.key] === "true" : values.fields[f.key] ?? "";
  }
  if (entry.resource) config[entry.resource.config_key] = values.resources;
  // Credential-backed (OAuth) data sources resolve a stored credential at run
  // time — the data source reads `credential_id` from its config.
  if (values.credentialId) config.credential_id = values.credentialId;
  return {
    name: values.name.trim(),
    type: entry.type_id,
    target_database: values.targetDatabase.trim() || sessionDatabase,
    target_table: values.targetTable.trim() || entry.default_target_table || "",
    schedule_ms: values.scheduleMs,
    config,
  };
}

/** The "Open graph" target for a data source, falling back to its type. */
export function graphNameForEntry(entry: CatalogEntry | undefined, type: string): string {
  return entry?.graph_name ?? type;
}

// ── Sync interval presets (clamped to the server's [100, 86_400_000] range) ─────

export const INTERVAL_PRESETS: { label: string; ms: number }[] = [
  { label: "Every minute", ms: 60_000 },
  { label: "Every 5 minutes", ms: 5 * 60_000 },
  { label: "Every 15 minutes", ms: 15 * 60_000 },
  { label: "Every 30 minutes", ms: 30 * 60_000 },
  { label: "Every hour", ms: 60 * 60_000 },
  { label: "Every 6 hours", ms: 6 * 60 * 60_000 },
  { label: "Every 24 hours", ms: 24 * 60 * 60_000 },
];

export function formatInterval(ms: number): string {
  const preset = INTERVAL_PRESETS.find((p) => p.ms === ms);
  if (preset) return preset.label;
  if (ms < 60_000) return `Every ${Math.round(ms / 1000)}s`;
  if (ms < 60 * 60_000) return `Every ${Math.round(ms / 60_000)}m`;
  return `Every ${Math.round(ms / (60 * 60_000))}h`;
}
