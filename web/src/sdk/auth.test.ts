import { beforeEach, expect, test, vi } from "vitest";
import { login, me, logout } from "./auth";

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

const endpoint = "http://localhost:8080";
const token = "session-token-abc";

// ── login ─────────────────────────────────────────────────────────────────────

test("login POSTs to /v1/auth/login with content-type but NO authorization header", async () => {
  mockFetch.mockResolvedValue(
    ok({ token, user: { username: "admin", role: "admin" }, expires_at: "2026-06-01T00:00:00Z" }),
  );
  const result = await login({ endpoint, username: "admin", password: "hunter2" });

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/login");
  expect(init.method).toBe("POST");
  expect(init.headers["content-type"]).toBe("application/json");
  // Must NOT send an Authorization header on the login request.
  expect(init.headers["authorization"]).toBeUndefined();
  expect(JSON.parse(init.body)).toMatchObject({ username: "admin", password: "hunter2" });
  expect(result.token).toBe(token);
  expect(result.user.username).toBe("admin");
  expect(result.user.role).toBe("admin");
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
  mockFetch.mockResolvedValue(ok({ token, user: { username: "admin", role: "admin" } }));
  await login({ endpoint: "http://localhost:8080/", username: "admin", password: "pw" });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/login");
});

// ── me ────────────────────────────────────────────────────────────────────────

test("me GETs /v1/auth/me with Authorization: Bearer header", async () => {
  mockFetch.mockResolvedValue(ok({ username: "admin", role: "admin" }));
  const user = await me({ endpoint, token });

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/me");
  expect(init.headers["authorization"]).toBe(`Bearer ${token}`);
  expect(user.username).toBe("admin");
});

test("me throws on 401", async () => {
  mockFetch.mockResolvedValue(new Response("no", { status: 401 }));
  await expect(me({ endpoint, token })).rejects.toThrow(/unauthorized/i);
});

// ── logout ────────────────────────────────────────────────────────────────────

test("logout POSTs to /v1/auth/logout with Authorization: Bearer header", async () => {
  mockFetch.mockResolvedValue(new Response(null, { status: 204 }));
  await logout({ endpoint, token });

  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:8080/v1/auth/logout");
  expect(init.method).toBe("POST");
  expect(init.headers["authorization"]).toBe(`Bearer ${token}`);
});

test("logout resolves on 204", async () => {
  mockFetch.mockResolvedValue(new Response(null, { status: 204 }));
  await expect(logout({ endpoint, token })).resolves.toBeUndefined();
});
