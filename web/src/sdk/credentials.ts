// Typed client for the kyma credentials store (`/v1/credentials/*`).
// Mirrors the per-module fetch-helper convention used by `sdk/connectors.ts`.

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

type BaseArgs = { endpoint: string; token: string };

function headers(token: string): Record<string, string> {
  return { authorization: `Bearer ${token}`, "content-type": "application/json" };
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

async function handleEmpty(res: Response): Promise<void> {
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
}

interface ListEnvelope {
  items: CredentialSummary[];
}

// ── API ───────────────────────────────────────────────────────────────────────

export async function listCredentials(args: BaseArgs): Promise<CredentialSummary[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/credentials`, { headers: headers(args.token) });
  const body = await handleResponse<ListEnvelope>(res);
  return body.items ?? [];
}

export async function createCredential(
  args: BaseArgs & { body: CreateCredentialBody },
): Promise<CredentialSummary> {
  const res = await fetch(`${base(args.endpoint)}/v1/credentials`, {
    method: "POST",
    headers: headers(args.token),
    body: JSON.stringify(args.body),
  });
  return handleResponse<CredentialSummary>(res);
}

export async function getCredential(
  args: BaseArgs & { id: string },
): Promise<CredentialSummary> {
  const res = await fetch(`${base(args.endpoint)}/v1/credentials/${args.id}`, {
    headers: headers(args.token),
  });
  return handleResponse<CredentialSummary>(res);
}

export async function deleteCredential(args: BaseArgs & { id: string }): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/credentials/${args.id}`, {
    method: "DELETE",
    headers: headers(args.token),
  });
  return handleEmpty(res);
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
