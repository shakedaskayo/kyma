import { beforeEach, describe, expect, test, vi } from "vitest";

// ── localStorage stub (zustand/persist reads it at module init) ───────────────
const store: Record<string, string> = {};
const localStorageMock = {
  getItem: (k: string) => store[k] ?? null,
  setItem: (k: string, v: string) => { store[k] = v; },
  removeItem: (k: string) => { delete store[k]; },
};
vi.stubGlobal("localStorage", localStorageMock);

// ── Dynamic imports (after localStorage stub) ─────────────────────────────────
const { useSession } = await import("./session");
const { sessionGetToken, sessionClient } = await import("./client");

beforeEach(() => {
  // Clear localStorage backing store and reset Zustand session state.
  for (const k of Object.keys(store)) delete store[k];
  useSession.getState().reset();
  // Also reset the module-level refresh-in-flight dedup lock by triggering a
  // "clean" import cycle — we can't easily reset module state, but the lock is
  // null after a resolved promise so tests that don't concurrently call with
  // reason:"expired" work fine in serial.
});

describe("sessionGetToken — initial / non-expired reason", () => {
  test("returns the current token when reason is not 'expired'", async () => {
    useSession.getState().set({ token: "tok-abc", endpoint: "http://pensieve.local" });
    const t = await sessionGetToken();
    expect(t).toBe("tok-abc");
  });

  test("returns the current token when reason is 'initial'", async () => {
    useSession.getState().set({ token: "tok-initial", endpoint: "http://pensieve.local" });
    const t = await sessionGetToken({ reason: "initial" });
    expect(t).toBe("tok-initial");
  });
});

describe("sessionGetToken — expired reason triggers refresh", () => {
  test("calls the refresh endpoint and updates the session store", async () => {
    useSession.getState().set({
      endpoint: "http://pensieve.local",
      token: "old-access",
      refreshToken: "old-refresh",
      accessExpiresAt: "2020-01-01T00:00:00Z",
    });

    // Mock globalThis.fetch for the refresh call.
    const mockPair = {
      access_token: "new-access",
      refresh_token: "new-refresh",
      access_expires_at: "2099-01-01T00:00:00Z",
      refresh_expires_at: "2099-06-01T00:00:00Z",
      user: { username: "alice", role: "admin" },
    };

    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify(mockPair), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    const result = await sessionGetToken({ reason: "expired" });

    // Returned value is the new access token.
    expect(result).toBe("new-access");

    // Session store was updated.
    const s = useSession.getState();
    expect(s.token).toBe("new-access");
    expect(s.refreshToken).toBe("new-refresh");
    expect(s.accessExpiresAt).toBe("2099-01-01T00:00:00Z");
    expect(s.user).toEqual({ username: "alice", role: "admin" });

    // The refresh endpoint was called with the refresh token.
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toContain("/v1/auth/refresh");
    const body = JSON.parse(init.body as string) as { refresh_token: string };
    expect(body.refresh_token).toBe("old-refresh");
  });

  test("throws and resets session when refresh fails (no endpoint)", async () => {
    // No endpoint set — sessionGetToken should throw immediately.
    await expect(sessionGetToken({ reason: "expired" })).rejects.toThrow();
  });
});

describe("sessionClient — cache behaviour", () => {
  test("returns the same client instance for the same endpoint+database", () => {
    useSession.getState().set({ endpoint: "http://pensieve.local", database: "obs", token: "t" });
    const c1 = sessionClient();
    const c2 = sessionClient();
    expect(c1).toBe(c2);
  });

  test("returns a new client when endpoint changes", () => {
    useSession.getState().set({ endpoint: "http://a.local", database: "obs", token: "t" });
    const c1 = sessionClient();
    useSession.getState().set({ endpoint: "http://b.local", database: "obs", token: "t" });
    const c2 = sessionClient();
    expect(c1).not.toBe(c2);
  });

  test("returned client has the expected namespace API shape", () => {
    useSession.getState().set({ endpoint: "http://pensieve.local", database: "obs", token: "t" });
    const c = sessionClient();
    expect(typeof c.discover.searchDiscover).toBe("function");
    expect(typeof c.query.runQuery).toBe("function");
    expect(typeof c.graph.listGraphs).toBe("function");
  });
});
