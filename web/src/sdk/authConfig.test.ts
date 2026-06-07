import { beforeEach, expect, test, vi } from "vitest";
import { fetchAuthConfig } from "./authConfig";

const mockFetch = vi.fn();
beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
});

function ok(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

test("fetchAuthConfig GETs /v1/auth/config unauthenticated", async () => {
  mockFetch.mockResolvedValue(ok({ provider: "password" }));
  const cfg = await fetchAuthConfig("http://localhost:8080/");

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/config");
  expect(init?.headers?.authorization).toBeUndefined();
  expect(cfg.provider).toBe("password");
});

test("fetchAuthConfig surfaces supabase mode with url, anon key, providers", async () => {
  mockFetch.mockResolvedValue(
    ok({
      provider: "supabase",
      supabase_url: "https://proj.supabase.co",
      supabase_anon_key: "anon-key",
      oauth_providers: ["google", "github"],
    }),
  );
  const cfg = await fetchAuthConfig("http://localhost:8080");
  expect(cfg.provider).toBe("supabase");
  if (cfg.provider === "supabase") {
    expect(cfg.supabase_url).toBe("https://proj.supabase.co");
    expect(cfg.supabase_anon_key).toBe("anon-key");
    expect(cfg.oauth_providers).toEqual(["google", "github"]);
  }
});

test("fetchAuthConfig falls back to password mode on older servers (404)", async () => {
  mockFetch.mockResolvedValue(new Response("not found", { status: 404 }));
  const cfg = await fetchAuthConfig("http://localhost:8080");
  expect(cfg.provider).toBe("password");
});
