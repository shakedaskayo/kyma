// App-level KymaClient bound to the live session store. Token rotation and
// dead-session redirects reuse the app's refresh flow via getToken.
import { createKymaClient, refresh, KymaAuthError, type KymaClient } from "@kyma-ai/client";
import { useSession } from "./session";

let cached: { key: string; client: KymaClient } | null = null;
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
    if (!s.endpoint || !s.refreshToken) throw new KymaAuthError(401, "no session");
    try {
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
 * Returns the session-level KymaClient, cached by endpoint+database.
 * Token is always read from the live Zustand store — no stale closures.
 */
export function sessionClient(): KymaClient {
  const { endpoint, database } = useSession.getState();
  const key = `${endpoint}|${database ?? ""}`;
  if (cached?.key === key) return cached.client;
  const client = createKymaClient({
    endpoint,
    database,
    auth: {
      getToken: sessionGetToken,
    },
  });
  cached = { key, client };
  return client;
}
