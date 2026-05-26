import type { ComponentType } from "react";
import { GitBranch } from "lucide-react";
import type { CreateConnectorBody } from "@/sdk/connectors";

// Declarative connector-kind registry. v1 ships only GitHub (PAT auth), but the
// wizard, config form, and detail view are all driven from these defs so a new
// connector (OAuth GitHub, GitLab, Notion, …) slots in by adding one entry —
// no UI rewiring.

export type AuthMode = "pat" | "oauth";

/** A schema-driven config field rendered by ConnectorConfigForm. */
export interface KindField {
  /** key under the connector `config` object */
  key: string;
  label: string;
  /** secret fields render as a password input and are kept out of caches/logs */
  type: "text" | "secret";
  placeholder?: string;
  required?: boolean;
  help?: string;
}

/** Inputs the wizard collects before calling `buildCreateBody`. */
export interface KindFormValues {
  name: string;
  scheduleMs: number;
  /** field key → value, e.g. { token: "ghp_…" } */
  fields: Record<string, string>;
  /** target db/table (advanced); fall back to kind defaults if blank */
  targetDatabase: string;
  targetTable: string;
  /** resource selection (e.g. selected repos "owner/name") */
  resources: string[];
}

export interface ConnectorKindDef {
  id: string;
  label: string;
  /** connector `type` sent to the backend */
  type: string;
  icon: ComponentType<{ className?: string }>;
  auth: { mode: AuthMode };
  /** the secret/config fields collected on the config step */
  fields: KindField[];
  /** whether the wizard shows a resource-selection step (repo picker) */
  resourceStep: boolean;
  /** the secret field key whose value unlocks the resource step (the PAT) */
  resourceTokenField?: string;
  /** default target db is overridden by the session database at build time */
  defaultTargetTable: string;
  /** sensible default sync interval (ms) */
  defaultScheduleMs: number;
  /** assemble the POST /v1/connectors body */
  buildCreateBody: (values: KindFormValues, sessionDatabase: string) => CreateConnectorBody;
  /** the registered graph name for this connector (for the "Open graph" link) */
  graphNameFor: (detail?: { type: string }) => string;
}

export const githubKind: ConnectorKindDef = {
  id: "github",
  label: "GitHub",
  type: "github",
  icon: GitBranch,
  auth: { mode: "pat" },
  fields: [
    {
      key: "token",
      label: "Personal access token",
      type: "secret",
      placeholder: "ghp_…",
      required: true,
      help: "A classic or fine-grained PAT with repo read access. Stored encrypted; never shown again.",
    },
  ],
  resourceStep: true,
  resourceTokenField: "token",
  defaultTargetTable: "github_nodes",
  defaultScheduleMs: 5 * 60_000,
  buildCreateBody: (values, sessionDatabase) => ({
    name: values.name.trim(),
    type: "github",
    target_database: values.targetDatabase.trim() || sessionDatabase,
    target_table: values.targetTable.trim() || "github_nodes",
    schedule_ms: values.scheduleMs,
    config: {
      token: values.fields.token ?? "",
      repos: values.resources,
      modules: {
        repos: true,
        branches: true,
        pulls: true,
        issues: true,
        contributors: true,
        codebase: true,
      },
    },
  }),
  graphNameFor: () => "github",
};

export const CONNECTOR_KINDS: ConnectorKindDef[] = [githubKind];

export function kindByType(type: string): ConnectorKindDef | undefined {
  return CONNECTOR_KINDS.find((k) => k.type === type);
}

/** The "Open graph" target for a connector, falling back to the kind default. */
export function graphNameForType(type: string): string {
  return kindByType(type)?.graphNameFor({ type }) ?? type;
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
