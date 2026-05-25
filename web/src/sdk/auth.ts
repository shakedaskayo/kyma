// Typed client for kyma auth endpoints (`/v1/auth/*`).
// `login` is unauthenticated (no Authorization header).
// `me` and `logout` require a Bearer token.

export interface AuthUser {
  username: string;
  role: string;
}

export interface LoginResult {
  token: string;
  user: AuthUser;
  expires_at?: string;
}

function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}

function bearerHeaders(token: string): Record<string, string> {
  return {
    authorization: `Bearer ${token}`,
    "content-type": "application/json",
  };
}

async function handleResponse<T>(res: Response): Promise<T> {
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
}): Promise<LoginResult> {
  const res = await fetch(`${base(args.endpoint)}/v1/auth/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username: args.username, password: args.password }),
  });
  return handleResponse<LoginResult>(res);
}

/**
 * GET /v1/auth/me — requires Bearer token.
 */
export async function me(args: { endpoint: string; token: string }): Promise<AuthUser> {
  const res = await fetch(`${base(args.endpoint)}/v1/auth/me`, {
    headers: bearerHeaders(args.token),
  });
  return handleResponse<AuthUser>(res);
}

/**
 * POST /v1/auth/logout — requires Bearer token. Returns void (204).
 */
export async function logout(args: { endpoint: string; token: string }): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/auth/logout`, {
    method: "POST",
    headers: bearerHeaders(args.token),
  });
  if (res.status === 204) return;
  await handleResponse<void>(res);
}
