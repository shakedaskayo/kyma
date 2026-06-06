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

const { createApiToken, listApiTokens, revokeApiToken } = await import("./tokens");
const { useSession } = await import("./session");

const mockFetch = vi.fn();
beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
  useSession.getState().set({ endpoint: "http://localhost:8080", token: "tok-123" });
});

function ok(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

test("createApiToken POSTs name+role with bearer auth and returns the raw token", async () => {
  mockFetch.mockResolvedValue(
    ok({ token: "raw-once", id: "abcd", name: "ci-bot", role: "write" }, 201),
  );
  const minted = await createApiToken({ name: "ci-bot", role: "write" });

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/tokens");
  expect(init.method).toBe("POST");
  expect(new Headers(init.headers).get("authorization")).toBe("Bearer tok-123");
  expect(JSON.parse(init.body)).toMatchObject({ name: "ci-bot", role: "write" });
  expect(minted.token).toBe("raw-once");
  expect(minted.role).toBe("write");
});

test("listApiTokens GETs the token list", async () => {
  mockFetch.mockResolvedValue(
    ok({
      tokens: [
        { id: "abcd", name: "ci-bot", role: "write", created_at: "2026-06-06T00:00:00Z", revoked: false },
      ],
    }),
  );
  const tokens = await listApiTokens();
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/tokens");
  expect(tokens).toHaveLength(1);
  expect(tokens[0].name).toBe("ci-bot");
});

test("revokeApiToken DELETEs by id and resolves on 204", async () => {
  mockFetch.mockResolvedValue(new Response(null, { status: 204 }));
  await revokeApiToken("abcd");
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/tokens/abcd");
  expect(init.method).toBe("DELETE");
});
