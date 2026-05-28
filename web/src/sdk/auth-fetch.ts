// Global fetch wrapper implementing transparent session refresh.
//
// On a 401 from a kyma API call, it runs a single (deduped) refresh-token
// exchange and retries the request once with the new access token. If refresh
// fails (refresh token expired/revoked, or none), it clears the session and
// redirects to /login. Auth endpoints (login/refresh) are excluded so a failed
// credential never triggers a refresh loop.
//
// Installed once at startup (see main.tsx). All SDKs keep using plain `fetch`
// with `Authorization: Bearer <session.token>`, so they gain session handling
// without any per-SDK changes.

import { useSession } from "./session";
import { refresh } from "./auth";

let installed = false;
let refreshInFlight: Promise<boolean> | null = null;

function isAuthEndpoint(url: string): boolean {
  return url.includes("/v1/auth/login") || url.includes("/v1/auth/refresh");
}

function urlOf(input: RequestInfo | URL): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.toString();
  return input.url;
}

/** Exchange the refresh token for a new pair. Deduped across concurrent 401s. */
function refreshOnce(): Promise<boolean> {
  if (refreshInFlight) return refreshInFlight;
  refreshInFlight = (async () => {
    const { endpoint, refreshToken } = useSession.getState();
    if (!endpoint || !refreshToken) return false;
    try {
      const pair = await refresh({ endpoint, refreshToken });
      useSession.getState().set({
        token: pair.access_token,
        refreshToken: pair.refresh_token,
        accessExpiresAt: pair.access_expires_at,
        user: pair.user,
      });
      return true;
    } catch {
      return false;
    }
  })();
  refreshInFlight.finally(() => {
    refreshInFlight = null;
  });
  return refreshInFlight;
}

/**
 * Authenticated fetch against the configured kyma endpoint.
 *
 * Accepts a path (e.g. "/v1/explore/search") or absolute URL. Paths are
 * resolved against the current session's `endpoint`. The session's access
 * token is attached as `Authorization: Bearer …`. Other headers from `init`
 * are preserved.
 *
 * The installed global fetch wrapper transparently handles 401 -> refresh -> retry.
 */
export async function authFetch(
  path: string,
  init?: RequestInit,
): Promise<Response> {
  const { endpoint, token } = useSession.getState();
  const url = path.startsWith("http")
    ? path
    : `${endpoint.replace(/\/$/, "")}${path.startsWith("/") ? path : `/${path}`}`;
  const headers = new Headers(init?.headers);
  if (token && !headers.has("authorization")) {
    headers.set("authorization", `Bearer ${token}`);
  }
  return fetch(url, { ...init, headers });
}

/** Install the global fetch wrapper. Idempotent. */
export function installAuthFetch(): void {
  if (installed || typeof window === "undefined") return;
  installed = true;
  const realFetch = window.fetch.bind(window);

  window.fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const res = await realFetch(input, init);

    const { endpoint } = useSession.getState();
    const url = urlOf(input);
    const isKymaApi = Boolean(endpoint) && url.startsWith(endpoint) && !isAuthEndpoint(url);
    if (res.status !== 401 || !isKymaApi) return res;

    const refreshed = await refreshOnce();
    if (!refreshed) {
      // Session is dead — clear it and bounce to login (preserving where we were).
      useSession.getState().reset();
      if (!window.location.pathname.startsWith("/login")) {
        const next = encodeURIComponent(window.location.pathname + window.location.search);
        window.location.assign(`/login?next=${next}`);
      }
      return res;
    }

    // Retry once with the refreshed access token.
    const token = useSession.getState().token;
    const headers = new Headers(
      init?.headers ??
        (typeof input === "object" && "headers" in input ? (input as Request).headers : undefined),
    );
    headers.set("authorization", `Bearer ${token}`);
    return realFetch(input, { ...init, headers });
  };
}
