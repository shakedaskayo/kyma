// API-token management (`/v1/auth/tokens`) — long-lived tokens for the CLI,
// MCP clients, and CI. The raw token is returned exactly once on creation.

import { authFetch } from "./auth-fetch";

export interface MintedToken {
  /** The raw bearer token — shown once, unrecoverable afterwards. */
  token: string;
  id: string;
  name: string | null;
  role: string;
  expires_at: string | null;
}

export interface ApiTokenEntry {
  id: string;
  name: string | null;
  role: string;
  created_at: string;
  last_used_at: string | null;
  expires_at: string | null;
  revoked: boolean;
}

async function jsonOrThrow<T>(res: Response): Promise<T> {
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

/** Mint a long-lived API token. Role is capped server-side at the caller's. */
export async function createApiToken(args: {
  name?: string;
  role?: string;
  expires_days?: number;
}): Promise<MintedToken> {
  const res = await authFetch("/v1/auth/tokens", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(args),
  });
  return jsonOrThrow<MintedToken>(res);
}

/** List API tokens (raw values are unrecoverable). */
export async function listApiTokens(): Promise<ApiTokenEntry[]> {
  const res = await authFetch("/v1/auth/tokens");
  const body = await jsonOrThrow<{ tokens: ApiTokenEntry[] }>(res);
  return body.tokens;
}

/** Revoke an API token by id. Resolves on 204; throws otherwise. */
export async function revokeApiToken(id: string): Promise<void> {
  const res = await authFetch(`/v1/auth/tokens/${id}`, { method: "DELETE" });
  if (res.status === 204) return;
  await jsonOrThrow<void>(res);
}
