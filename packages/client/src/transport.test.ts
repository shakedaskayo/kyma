import { describe, expect, it, vi } from "vitest";
import { createTransport } from "./transport";
import { KymaAuthError } from "./errors";

const ok = (body: unknown) =>
  new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });

describe("createTransport", () => {
  it("resolves paths against endpoint and attaches bearer + x-database", async () => {
    const fetchMock = vi.fn().mockResolvedValue(ok({}));
    const t = createTransport({
      endpoint: "https://kyma.example.com/",
      auth: { token: "tok-1" },
      database: "prod",
      fetch: fetchMock,
    });
    await t.request("/v1/graph");
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe("https://kyma.example.com/v1/graph");
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
    const getToken = vi.fn().mockResolvedValueOnce("t1").mockResolvedValueOnce("t1").mockResolvedValue("t2");
    const t = createTransport({ endpoint: "https://k", auth: { getToken }, fetch: fetchMock });
    const [r1, r2] = await Promise.all([t.request("/v1/a"), t.request("/v1/b")]);
    expect(r1.status).toBe(200);
    expect(r2.status).toBe(200);
    expect(getToken).toHaveBeenCalledTimes(2);
    expect(getToken).toHaveBeenLastCalledWith({ reason: "expired" });
  });

  it("throws KymaAuthError when retry after re-mint still 401s", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("nope", { status: 401 }));
    const t = createTransport({
      endpoint: "https://k",
      auth: { getToken: async () => "always-bad" },
      fetch: fetchMock,
    });
    await expect(t.request("/v1/a")).rejects.toBeInstanceOf(KymaAuthError);
  });

  it("throws KymaAuthError immediately on 401 with static token (no re-mint possible)", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("nope", { status: 401 }));
    const t = createTransport({ endpoint: "https://k", auth: { token: "x" }, fetch: fetchMock });
    await expect(t.request("/v1/a")).rejects.toBeInstanceOf(KymaAuthError);
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
});
