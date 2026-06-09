// Typed client for kyma auth endpoints (`/v1/auth/*`).
// `login` and `refresh` are unauthenticated — they use plain fetch + endpoint.
// `me` and `logout` require a Bearer token and use KymaTransport.

import type { KymaTransport } from "./transport";
import { errorFromResponse } from "./errors";

export interface AuthUser {
  username: string;
  role: string;
}

/** Access + refresh token pair returned by login and refresh. */
export interface TokenPair {
  access_token: string;
  refresh_token: string;
  access_expires_at: string;
  refresh_expires_at: string;
  user: AuthUser;
}

function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}

async function handleUnauthResponse<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) {
    // Try to extract a server-provided error message.
    const body = await res.json().catch(() => null) as { error?: { message?: string } } | null;
    const msg = body?.error?.message ?? `unauthorized (${res.status})`;
    throw new Error(msg);
  }
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

/**
 * POST /v1/auth/login — unauthenticated.
 * Returns the session token + user on success; throws on 401.
 */
export async function login(args: {
  endpoint: string;
  username: string;
  password: string;
}): Promise<TokenPair> {
  const res = await globalThis.fetch(`${base(args.endpoint)}/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: args.username, password: args.password }),
  });
  return handleUnauthResponse<TokenPair>(res);
}

/**
 * POST /v1/auth/refresh — exchange a refresh token for a rotated pair.
 * Unauthenticated (the refresh token is the credential). Throws on 401.
 */
export async function refresh(args: {
  endpoint: string;
  refreshToken: string;
}): Promise<TokenPair> {
  const res = await globalThis.fetch(`${base(args.endpoint)}/v1/auth/refresh`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ refresh_token: args.refreshToken }),
  });
  return handleUnauthResponse<TokenPair>(res);
}

/**
 * GET /v1/auth/me — requires Bearer token.
 */
export async function me(t: KymaTransport): Promise<AuthUser> {
  const res = await t.request("/v1/auth/me");
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<AuthUser>;
}

/**
 * POST /v1/auth/logout — requires Bearer token. Returns void (204).
 */
export async function logout(t: KymaTransport): Promise<void> {
  const res = await t.request("/v1/auth/logout", { method: "POST" });
  if (res.status === 204) return;
  if (!res.ok) throw await errorFromResponse(res);
}
