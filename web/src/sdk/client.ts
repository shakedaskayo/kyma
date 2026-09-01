// App-level PensieveClient bound to the live session store. Token rotation and
// dead-session redirects reuse the app's refresh flow via getToken.
import { createPensieveClient, refresh, PensieveAuthError, type PensieveClient } from "@pensieve-ai/client";
import { useSession } from "./session";

let cached: { key: string; client: PensieveClient } | null = null;
let refreshInFlight: Promise<string> | null = null;

/**
 * Exchange the refresh token for a new pair, deduped across concurrent 401s.
 * Updates the session store on success; clears it and redirects on failure.
 */
export async function sessionGetToken(opts?: { reason?: string }): Promise<string> {
  if (opts?.reason !== "expired") {
    return useSession.getState().token;
  }
  if (refreshInFlight) return refreshInFlight;
  refreshInFlight = (async () => {
    const s = useSession.getState();
    try {
      // Supabase sessions refresh through supabase-js (which owns the refresh
      // token); password sessions exchange pensieve's refresh token directly.
      if (s.provider === "supabase") {
        const { refreshSupabaseToken } = await import("./supabase");
        const tok = await refreshSupabaseToken();
        if (!tok) throw new PensieveAuthError(401, "supabase refresh failed");
        return tok;
      }
      if (!s.endpoint || !s.refreshToken) throw new PensieveAuthError(401, "no session");
      const pair = await refresh({ endpoint: s.endpoint, refreshToken: s.refreshToken });
      useSession.getState().set({
        token: pair.access_token,
        refreshToken: pair.refresh_token,
        accessExpiresAt: pair.access_expires_at,
        user: pair.user,
      });
      return pair.access_token;
    } catch (e) {
      useSession.getState().reset();
      if (typeof window !== "undefined" && !window.location.pathname.startsWith("/login")) {
        const next = encodeURIComponent(window.location.pathname + window.location.search);
        window.location.assign(`/login?next=${next}`);
      }
      throw e;
    } finally {
      refreshInFlight = null;
    }
  })();
  return refreshInFlight;
}

/**
 * Returns the session-level PensieveClient, cached by endpoint+database.
 * Token is always read from the live Zustand store — no stale closures.
 */
export function sessionClient(): PensieveClient {
  const { endpoint, database } = useSession.getState();
  const key = `${endpoint}|${database ?? ""}`;
  if (cached?.key === key) return cached.client;
  const client = createPensieveClient({
    endpoint,
    database,
    auth: {
      getToken: sessionGetToken,
    },
  });
  cached = { key, client };
  return client;
}
