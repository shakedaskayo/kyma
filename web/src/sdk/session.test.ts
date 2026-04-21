import { beforeEach, expect, test } from "vitest";
import { useSession } from "./session";

beforeEach(() => { localStorage.clear(); useSession.getState().reset(); });

test("stores and retrieves endpoint + token", () => {
  useSession.getState().set({ endpoint: "http://localhost:8080", token: "abc" });
  const s = useSession.getState();
  expect(s.endpoint).toBe("http://localhost:8080");
  expect(s.token).toBe("abc");
});

test("is configured only when both endpoint and token are present", () => {
  expect(useSession.getState().isConfigured()).toBe(false);
  useSession.getState().set({ endpoint: "http://x", token: "" });
  expect(useSession.getState().isConfigured()).toBe(false);
  useSession.getState().set({ endpoint: "http://x", token: "t" });
  expect(useSession.getState().isConfigured()).toBe(true);
});
