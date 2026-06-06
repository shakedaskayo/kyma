// Provider-aware 401 recovery in the global fetch wrapper: supabase sessions
// refresh through supabase-js (mocked here), NOT kyma's /v1/auth/refresh.
import { beforeEach, expect, test, vi } from "vitest";

// zustand/persist reads localStorage at module init — stub it first.
const store: Record<string, string> = {};
vi.stubGlobal("localStorage", {
  getItem: (k: string) => store[k] ?? null,
  setItem: (k: string, v: string) => {
    store[k] = v;
  },
  removeItem: (k: string) => {
    delete store[k];
  },
});

vi.mock("./supabase", () => ({
  refreshSupabaseToken: vi.fn(async () => {
    const { useSession } = await import("./session");
    useSession.getState().set({ token: "sb-refreshed" });
    return "sb-refreshed";
  }),
}));

const { useSession } = await import("./session");
const { refreshSupabaseToken } = await import("./supabase");
const { installAuthFetch } = await import("./auth-fetch");

const realFetch = vi.fn();

beforeEach(() => {
  vi.stubGlobal("fetch", realFetch);
  realFetch.mockReset();
  vi.mocked(refreshSupabaseToken).mockClear();
  useSession.getState().reset();
});

test("supabase session: 401 → refresh via supabase-js → retry with new token", async () => {
  useSession.getState().set({
    endpoint: "http://localhost:8080",
    token: "sb-stale",
    provider: "supabase",
  });
  installAuthFetch();

  realFetch
    .mockResolvedValueOnce(new Response("unauthorized", { status: 401 }))
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { "content-type": "application/json" },
      }),
    );

  const res = await window.fetch("http://localhost:8080/v1/query", {
    headers: { authorization: "Bearer sb-stale" },
  });

  expect(res.status).toBe(200);
  expect(refreshSupabaseToken).toHaveBeenCalledTimes(1);
  // kyma's own refresh endpoint must NOT be called in supabase mode.
  const urls = realFetch.mock.calls.map(([u]) => String(u));
  expect(urls.some((u) => u.includes("/v1/auth/refresh"))).toBe(false);
  // The retry carried the refreshed token.
  const retryHeaders = new Headers(realFetch.mock.calls[1][1]?.headers);
  expect(retryHeaders.get("authorization")).toBe("Bearer sb-refreshed");
});
