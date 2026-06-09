import { beforeEach, expect, test, vi } from "vitest";
import { login, me, logout, refresh } from "./auth";
import { createTransport } from "./transport";

const mockFetch = vi.fn();
beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
});

function makeTransport(fetchMock: typeof fetch) {
  return createTransport({ endpoint: "http://localhost:8080", auth: { token: "session-token-abc" }, fetch: fetchMock });
}

function ok(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const endpoint = "http://localhost:8080";
const token = "session-token-abc";

function pair(overrides: Record<string, unknown> = {}) {
  return {
    access_token: "access-abc",
    refresh_token: "refresh-xyz",
    access_expires_at: "2026-06-01T00:00:00Z",
    refresh_expires_at: "2026-07-01T00:00:00Z",
    user: { username: "admin", role: "admin" },
    ...overrides,
  };
}

// ── login ─────────────────────────────────────────────────────────────────────

test("login POSTs to /v1/auth/login with content-type but NO authorization header", async () => {
  mockFetch.mockResolvedValue(ok(pair()));
  const result = await login({ endpoint, username: "admin", password: "hunter2" });

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/login");
  expect(init.method).toBe("POST");
  expect(init.headers["content-type"]).toBe("application/json");
  // Must NOT send an Authorization header on the login request.
  expect(init.headers["authorization"]).toBeUndefined();
  expect(JSON.parse(init.body)).toMatchObject({ username: "admin", password: "hunter2" });
  expect(result.access_token).toBe("access-abc");
  expect(result.refresh_token).toBe("refresh-xyz");
  expect(result.user.username).toBe("admin");
  expect(result.user.role).toBe("admin");
});

// ── refresh ─────────────────────────────────────────────────────────────────────

test("refresh POSTs the refresh_token to /v1/auth/refresh and returns a rotated pair", async () => {
  mockFetch.mockResolvedValue(ok(pair({ access_token: "access-2", refresh_token: "refresh-2" })));
  const result = await refresh({ endpoint, refreshToken: "refresh-xyz" });

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/refresh");
  expect(init.method).toBe("POST");
  expect(init.headers["authorization"]).toBeUndefined();
  expect(JSON.parse(init.body)).toEqual({ refresh_token: "refresh-xyz" });
  expect(result.access_token).toBe("access-2");
  expect(result.refresh_token).toBe("refresh-2");
});

test("refresh throws on 401 (expired/revoked refresh token)", async () => {
  mockFetch.mockResolvedValue(new Response("{}", { status: 401 }));
  await expect(refresh({ endpoint, refreshToken: "stale" })).rejects.toThrow(/unauthorized/i);
});

test("login throws on 401 with server error message", async () => {
  mockFetch.mockResolvedValue(
    new Response(JSON.stringify({ error: { code: "invalid_credentials", message: "bad password" } }), {
      status: 401,
      headers: { "content-type": "application/json" },
    }),
  );
  await expect(login({ endpoint, username: "admin", password: "wrong" })).rejects.toThrow("bad password");
});

test("login throws on 401 without a body message", async () => {
  mockFetch.mockResolvedValue(new Response("{}", { status: 401 }));
  await expect(login({ endpoint, username: "admin", password: "wrong" })).rejects.toThrow(/unauthorized/i);
});

test("login strips trailing slash from endpoint", async () => {
  mockFetch.mockResolvedValue(ok(pair()));
  await login({ endpoint: "http://localhost:8080/", username: "admin", password: "pw" });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/login");
});

// ── me ────────────────────────────────────────────────────────────────────────

test("me GETs /v1/auth/me with Authorization: Bearer header", async () => {
  mockFetch.mockResolvedValue(ok({ username: "admin", role: "admin" }));
  const user = await me(makeTransport(mockFetch));

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/me");
  expect(init.headers.get("authorization")).toBe(`Bearer ${token}`);
  expect(user.username).toBe("admin");
});

test("me throws on 401", async () => {
  mockFetch.mockResolvedValue(new Response("no", { status: 401 }));
  await expect(me(makeTransport(mockFetch))).rejects.toThrow(/unauthorized|token rejected/i);
});

// ── logout ────────────────────────────────────────────────────────────────────

test("logout POSTs to /v1/auth/logout with Authorization: Bearer header", async () => {
  mockFetch.mockResolvedValue(new Response(null, { status: 204 }));
  await logout(makeTransport(mockFetch));

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/logout");
  expect(init.method).toBe("POST");
  expect(init.headers.get("authorization")).toBe(`Bearer ${token}`);
});

test("logout resolves on 204", async () => {
  mockFetch.mockResolvedValue(new Response(null, { status: 204 }));
  await expect(logout(makeTransport(mockFetch))).resolves.toBeUndefined();
});
