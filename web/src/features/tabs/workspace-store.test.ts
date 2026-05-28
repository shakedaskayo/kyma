import { beforeEach, expect, test } from "vitest";
import { useWorkspace, isTabDirty, migrateWorkspace } from "./workspace-store";

beforeEach(() => {
  useWorkspace.getState().resetAll();
});

test("newTab defaults to discover kind", () => {
  const id = useWorkspace.getState().newTab();
  const t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(t.kind).toBe("discover");
  if (t.kind !== "discover") throw new Error("type narrowing");
  expect(t.state.scope.kind).toBe("all");
  expect(t.state.search).toBe("");
  expect(t.state.pills).toEqual([]);
  expect(t.state.timeRange.preset).toBe("1h");
});

test("newTab with kind: query produces a query tab", () => {
  const id = useWorkspace.getState().newTab({ kind: "query" });
  const t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(t.kind).toBe("query");
  if (t.kind !== "query") throw new Error("type narrowing");
  expect(t.state.query).toBe("");
  expect(t.state.timeRange.preset).toBe("1h");
  expect(t.state.results.kind).toBe("idle");
});

test("updateQuery updates only the target query tab", () => {
  const a = useWorkspace.getState().newTab({ kind: "query" });
  const b = useWorkspace.getState().newTab({ kind: "query" });
  useWorkspace.getState().updateQuery(a, { query: "foo | take 1" });
  const { tabs } = useWorkspace.getState();
  const ta = tabs.find((t) => t.id === a)!;
  const tb = tabs.find((t) => t.id === b)!;
  if (ta.kind !== "query" || tb.kind !== "query") throw new Error("kind");
  expect(ta.state.query).toBe("foo | take 1");
  expect(tb.state.query).toBe("");
});

test("updateQuery is a no-op on a discover tab", () => {
  const d = useWorkspace.getState().newTab({ kind: "discover" });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (useWorkspace.getState().updateQuery as any)(d, { query: "should not apply" });
  const t = useWorkspace.getState().tabs.find((x) => x.id === d)!;
  expect(t.kind).toBe("discover");
  if (t.kind !== "discover") throw new Error("kind");
  expect((t.state as unknown as { query?: string }).query).toBeUndefined();
  expect(t.state.search).toBe("");
});

test("closeTab removes the tab and picks a new active when needed", () => {
  const a = useWorkspace.getState().newTab({ kind: "query" });
  const b = useWorkspace.getState().newTab({ kind: "query" });
  useWorkspace.getState().setActive(b);
  useWorkspace.getState().closeTab(b);
  expect(useWorkspace.getState().activeId).toBe(a);
});

test("isTabDirty reflects submittedQuery vs current query on query tabs", () => {
  const id = useWorkspace.getState().newTab({ kind: "query" });
  let t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(isTabDirty(t)).toBe(false);
  useWorkspace.getState().updateQuery(id, { query: "abc", submittedQuery: "abc" });
  t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(isTabDirty(t)).toBe(false);
  useWorkspace.getState().updateQuery(id, { query: "abc | take 1" });
  t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(isTabDirty(t)).toBe(true);
});

test("isTabDirty is always false on discover tabs", () => {
  const id = useWorkspace.getState().newTab({ kind: "discover" });
  const t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(isTabDirty(t)).toBe(false);
});

test("patchDiscover merges into a discover tab", () => {
  const id = useWorkspace.getState().newTab({ kind: "discover" });
  useWorkspace.getState().patchDiscover(id, { search: "hello" });
  const t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  if (t.kind !== "discover") throw new Error("kind");
  expect(t.state.search).toBe("hello");
  expect(t.state.scope.kind).toBe("all");
});

test("migrateWorkspace converts v1 flat tabs to v2 query tabs", () => {
  const legacy = {
    state: {
      tabs: [
        {
          id: "legacy-1",
          title: "old query 1",
          query: "table1 | take 10",
          timeRange: { preset: "6h" },
          results: { kind: "idle" },
          chart: {},
          submittedQuery: null,
        },
        {
          id: "legacy-2",
          title: "old query 2",
          query: "table2 | take 5",
          timeRange: { preset: "1h" },
          results: { kind: "idle" },
          chart: {},
          submittedQuery: "table2 | take 5",
        },
      ],
      activeId: "legacy-2",
    },
  };

  const migrated = migrateWorkspace(legacy, 1) as {
    state: { tabs: Array<{ id: string; kind: string; state: { query: string; submittedQuery: string | null } }> };
  };

  expect(migrated.state.tabs.length).toBe(2);
  for (const t of migrated.state.tabs) {
    expect(t.kind).toBe("query");
    expect(typeof t.state.query).toBe("string");
  }
  const t0 = migrated.state.tabs.find((t) => t.id === "legacy-1")!;
  const t1 = migrated.state.tabs.find((t) => t.id === "legacy-2")!;
  expect(t0.state.query).toBe("table1 | take 10");
  expect(t1.state.submittedQuery).toBe("table2 | take 5");
});

test("migrateWorkspace is a no-op for v2", () => {
  const v2 = {
    state: {
      tabs: [
        { id: "x", kind: "discover", state: { scope: { kind: "all" }, search: "", pills: [] } },
      ],
      activeId: "x",
    },
  };
  const migrated = migrateWorkspace(v2, 2);
  expect(migrated).toBe(v2);
});
