// Typed client for the kyma credentials store (`/v1/credentials/*`).

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

// ── Types (must match crates/kyma-core/src/credentials.rs) ────────────────────

export type CredentialKind =
  | "pat"
  | "basic"
  | "oauth2"
  | "url"
  | "aws_creds"
  | "api_key"
  | "github_app";

/** Tagged union of secret material — the `kind` discriminator picks the shape. */
export type CredentialValue =
  | { kind: "pat"; token: string }
  | { kind: "basic"; username: string; password: string }
  | { kind: "oauth2"; access_token: string; refresh_token?: string; expires_at?: string }
  | { kind: "url"; connection_string: string }
  | { kind: "aws_creds"; access_key_id: string; secret_access_key: string; session_token?: string }
  | { kind: "api_key"; value: string; header_name?: string }
  | { kind: "github_app"; app_id: string; installation_id: string; private_key_pem: string };

/** Public, non-secret view (no value, just a masked preview). */
export interface CredentialSummary {
  id: string;
  label: string;
  kind: CredentialKind;
  preview: string;
  metadata: Record<string, unknown>;
  created_at: string;
  updated_at: string;
}

export interface CreateCredentialBody {
  label: string;
  value: CredentialValue;
  metadata?: Record<string, unknown>;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

interface ListEnvelope {
  items: CredentialSummary[];
}

// ── API ───────────────────────────────────────────────────────────────────────

export async function listCredentials(t: KymaTransport): Promise<CredentialSummary[]> {
  const body = await handleResponse<ListEnvelope>(await t.request("/v1/credentials"));
  return body.items ?? [];
}

export async function createCredential(
  t: KymaTransport,
  args: { body: CreateCredentialBody },
): Promise<CredentialSummary> {
  return handleResponse<CredentialSummary>(
    await t.request("/v1/credentials", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.body),
    }),
  );
}

export async function getCredential(
  t: KymaTransport,
  args: { id: string },
): Promise<CredentialSummary> {
  return handleResponse<CredentialSummary>(await t.request(`/v1/credentials/${args.id}`));
}

export async function deleteCredential(t: KymaTransport, args: { id: string }): Promise<void> {
  const res = await t.request(`/v1/credentials/${args.id}`, { method: "DELETE" });
  if (!res.ok) throw await errorFromResponse(res);
}

// ── Presentation metadata ─────────────────────────────────────────────────────

/** Display label for a credential kind (UI-facing). */
export function credentialKindLabel(kind: string): string {
  switch (kind) {
    case "pat": return "Personal access token";
    case "basic": return "Basic auth";
    case "oauth2": return "OAuth 2.0";
    case "url": return "Connection URL";
    case "aws_creds": return "AWS access keys";
    case "api_key": return "API key";
    case "github_app": return "GitHub App";
    default: return kind;
  }
}
