import { beforeEach, expect, test, vi } from "vitest";

// Provide a minimal localStorage stub before importing the store,
// because zustand/persist tries to read it at module init time.
const store: Record<string, string> = {};
const localStorageMock = {
  getItem: (k: string) => store[k] ?? null,
  setItem: (k: string, v: string) => { store[k] = v; },
  removeItem: (k: string) => { delete store[k]; },
};
vi.stubGlobal("localStorage", localStorageMock);

// Import the store AFTER stubbing localStorage.
const { useSession } = await import("./session");

beforeEach(() => {
  // Clear backing store + reset zustand state.
  for (const k of Object.keys(store)) delete store[k];
  useSession.getState().reset();
});

test("stores and retrieves endpoint + token", () => {
  useSession.getState().set({ endpoint: "http://localhost:8080", token: "abc" });
  const s = useSession.getState();
  expect(s.endpoint).toBe("http://localhost:8080");
  expect(s.token).toBe("abc");
});

test("isConfigured is true when endpoint is set (token is optional for auth-disabled engines)", () => {
  expect(useSession.getState().isConfigured()).toBe(false);
  useSession.getState().set({ endpoint: "http://x", token: "" });
  expect(useSession.getState().isConfigured()).toBe(true);
  useSession.getState().set({ endpoint: "http://x", token: "t" });
  expect(useSession.getState().isConfigured()).toBe(true);
});

test("isAuthenticated is true only when token is present", () => {
  expect(useSession.getState().isAuthenticated()).toBe(false);
  useSession.getState().set({ endpoint: "http://x", token: "" });
  expect(useSession.getState().isAuthenticated()).toBe(false);
  useSession.getState().set({ endpoint: "http://x", token: "tok" });
  expect(useSession.getState().isAuthenticated()).toBe(true);
});

test("user field is null after reset", () => {
  useSession.getState().set({ endpoint: "http://x", token: "tok", user: { username: "admin", role: "admin" } });
  expect(useSession.getState().user?.username).toBe("admin");
  useSession.getState().reset();
  expect(useSession.getState().user).toBeNull();
});

test("set accepts user field", () => {
  useSession.getState().set({ user: { username: "alice", role: "read" } });
  expect(useSession.getState().user).toMatchObject({ username: "alice", role: "read" });
});
