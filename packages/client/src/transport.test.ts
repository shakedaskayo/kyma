import { describe, expect, it, vi } from "vitest";
import { createTransport } from "./transport";
import { PensieveAuthError } from "./errors";

const ok = (body: unknown) =>
  new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });

/** Build a minimal fake JWT with the given exp (seconds since epoch). */
function makeJwt(exp: number): string {
  const header = btoa(JSON.stringify({ alg: "HS256", typ: "JWT" })).replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_");
  const payload = btoa(JSON.stringify({ sub: "user", exp })).replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_");
  return `${header}.${payload}.fakesig`;
}

describe("createTransport", () => {
  it("resolves paths against endpoint and attaches bearer + x-database", async () => {
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({
      endpoint: "https://pensieve.example.com/",
      auth: { token: "tok-1" },
      database: "prod",
      fetch: fetchMock,
    });
    await t.request("/v1/graph");
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("https://pensieve.example.com/v1/graph");
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer tok-1");
    expect(new Headers(init.headers).get("x-database")).toBe("prod");
  });

  it("per-request database overrides the default", async () => {
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({ endpoint: "https://k", auth: { token: "t" }, database: "prod", fetch: fetchMock });
    await t.request("/v1/query", { database: "staging" });
    expect(new Headers(fetchMock.mock.calls[0][1].headers).get("x-database")).toBe("staging");
  });

  it("on 401 with getToken: re-mints once (single-flight) and retries", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(new Response("expired", { status: 401 }))
      .mockResolvedValueOnce(new Response("expired", { status: 401 }))
      .mockResolvedValue(ok({ fine: true }));
    // Fix: removed the dead second mockResolvedValueOnce("t1") — second call never consumed
    const getToken = vi.fn().mockResolvedValueOnce("t1").mockResolvedValue("t2");
    const t = createTransport({ endpoint: "https://k", auth: { getToken }, fetch: fetchMock });
    const [r1, r2] = await Promise.all([t.request("/v1/a"), t.request("/v1/b")]);
    expect(r1.status).toBe(200);
    expect(r2.status).toBe(200);
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(getToken).toHaveBeenLastCalledWith({ reason: "expired" });
  });

  it("throws PensieveAuthError when retry after re-mint still 401s", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("nope", { status: 401 }));
    const t = createTransport({
      endpoint: "https://k",
      auth: { getToken: async () => "always-bad" },
      fetch: fetchMock,
    });
    await expect(t.request("/v1/a")).rejects.toBeInstanceOf(PensieveAuthError);
  });

  it("throws PensieveAuthError immediately on 401 with static token (no re-mint possible)", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("nope", { status: 401 }));
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    await expect(t.request("/v1/a")).rejects.toBeInstanceOf(PensieveAuthError);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("converts HTML 200 from /v1/* into a 404 (legacy SPA fallback)", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response("<!doctype html>", { status: 200, headers: { "content-type": "text/html" } }),
    );
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    const res = await t.request("/v1/anything");
    expect(res.status).toBe(404);
  });

  it("does not throw for non-auth errors — returns the response for callers to map", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("boom", { status: 500 }));
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    const res = await t.request("/v1/a");
    expect(res.status).toBe(500);
  });

  it("propagates AbortSignal", async () => {
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    const ac = new AbortController();
    await t.request("/v1/a", { signal: ac.signal });
    expect(fetchMock.mock.calls[0][1].signal).toBe(ac.signal);
  });

  // Fix 1: Thundering-refresh guard for staggered 401s
  it("staggered 401: B's 401 with stale token reuses already-fresh token without extra mint", async () => {
    // fetch respects the token it receives: t1 → 401, t2 → 200
    const getToken = vi.fn().mockResolvedValueOnce("t1").mockResolvedValueOnce("t2");
    const fetchMock = vi.fn().mockImplementation((_url: string, init: RequestInit) => {
      const auth = new Headers(init.headers as HeadersInit).get("authorization");
      if (auth === "Bearer t1") return Promise.resolve(new Response("expired", { status: 401 }));
      return Promise.resolve(ok({ fine: true }));
    });
    const t = createTransport({ endpoint: "https://k", auth: { getToken }, fetch: fetchMock });

    // A 401s and re-mints to t2; wait for it to complete fully
    await t.request("/v1/a");

    // B's request: was originally sent with t1 (stale) — simulate by having fetch mock 401 on t1
    // At this point cachedToken = t2, so B's 401 retry should use t2 with no new mint
    const res = await t.request("/v1/b");
    expect(res.status).toBe(200);
    // Only 2 getToken calls total: "initial" (t1) + "expired" (t2)
    expect(getToken).toHaveBeenCalledTimes(2);
  });

  // Fix 2a: Proactive JWT refresh — token expiring soon triggers re-mint
  it("proactive refresh: JWT expiring in 30s triggers re-mint on next request", async () => {
    const expiringSoon = makeJwt(Math.floor(Date.now() / 1000) + 30);
    const freshToken = makeJwt(Math.floor(Date.now() / 1000) + 3600);
    const getToken = vi.fn().mockResolvedValueOnce(expiringSoon).mockResolvedValueOnce(freshToken);
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({ endpoint: "https://k", auth: { getToken }, fetch: fetchMock });

    await t.request("/v1/a"); // First request — gets expiringSoon, uses it
    await t.request("/v1/b"); // Second request — proactively re-mints because exp within 60s
    expect(getToken).toHaveBeenCalledTimes(2);
  });

  // Fix 2b: No proactive refresh when JWT is not near expiry
  it("proactive refresh: JWT expiring in 10 min does NOT re-mint", async () => {
    const longLived = makeJwt(Math.floor(Date.now() / 1000) + 600);
    const getToken = vi.fn().mockResolvedValueOnce(longLived);
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({ endpoint: "https://k", auth: { getToken }, fetch: fetchMock });

    await t.request("/v1/a");
    await t.request("/v1/b");
    expect(getToken).toHaveBeenCalledTimes(1);
  });

  // Fix 2c: Opaque (non-JWT) token — no proactive refresh
  it("proactive refresh: opaque token is never proactively refreshed", async () => {
    const getToken = vi.fn().mockResolvedValueOnce("opaque-token-no-dots");
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({ endpoint: "https://k", auth: { getToken }, fetch: fetchMock });

    await t.request("/v1/a");
    await t.request("/v1/b");
    expect(getToken).toHaveBeenCalledTimes(1);
  });

  // Fix 3: Preserve requestId on auth errors
  it("thrown PensieveAuthError on 401 includes requestId from x-request-id header", async () => {
    const fetchMock = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ error: "token expired" }), {
        status: 401,
        headers: { "content-type": "application/json", "x-request-id": "req-abc-123" },
      }),
    );
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    const err = await t.request("/v1/a").catch((e) => e);
    expect(err).toBeInstanceOf(PensieveAuthError);
    expect(err.requestId).toBe("req-abc-123");
  });
});
