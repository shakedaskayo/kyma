import { beforeEach, expect, test } from "vitest";
import { useWorkspace } from "./workspace-store";

beforeEach(() => { localStorage.clear(); useWorkspace.getState().resetAll(); });

test("newTab creates a fresh tab with empty query", () => {
  const id = useWorkspace.getState().newTab();
  const t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(t.query).toBe("");
  expect(t.timeRange.preset).toBe("1h");
});

test("setQuery updates only the target tab", () => {
  const a = useWorkspace.getState().newTab();
  const b = useWorkspace.getState().newTab();
  useWorkspace.getState().setQuery(a, "foo | take 1");
  const { tabs } = useWorkspace.getState();
  expect(tabs.find((t) => t.id === a)!.query).toBe("foo | take 1");
  expect(tabs.find((t) => t.id === b)!.query).toBe("");
});

test("closeTab removes the tab and picks a new active when needed", () => {
  const a = useWorkspace.getState().newTab();
  const b = useWorkspace.getState().newTab();
  useWorkspace.getState().setActive(b);
  useWorkspace.getState().closeTab(b);
  expect(useWorkspace.getState().activeId).toBe(a);
});
