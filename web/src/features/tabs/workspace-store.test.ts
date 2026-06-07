import { beforeEach, expect, test } from "vitest";
import {
  useWorkspace,
  isTabDirty,
  migrateWorkspace,
  dedupeTabs,
  findMatchingQueryTab,
  type Tab,
} from "./workspace-store";

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
  expect(t.state.columns).toEqual([]);
  expect(t.state.viewMode).toBe("stream");
  expect(t.state.timeRange.preset).toBe("1h");
  expect(t.state.live).toBe(false);
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

// --- dedupeTabs -------------------------------------------------------------

function queryTab(id: string, query: string, preset = "1h"): Tab {
  return {
    id,
    kind: "query",
    state: {
      title: `query ${id}`,
      query,
      timeRange: { preset: preset as "1h" },
      results: { kind: "idle" },
      chart: {},
      submittedQuery: null,
    },
  };
}

function discoverTab(id: string, search = ""): Tab {
  return {
    id,
    kind: "discover",
    state: {
      scope: { kind: "all" },
      search,
      timeRange: { preset: "1h" },
      visibleSources: null,
      selectedSource: null,
      columns: [],
      viewMode: "stream",
      live: false,
      results: { status: "idle", sources: new Map() },
    },
  };
}

test("dedupeTabs collapses query tabs with identical query + time range", () => {
  const tabs = [queryTab("a", "t1 | take 50"), queryTab("b", "t1 | take 50"), queryTab("c", "t2 | count")];
  const out = dedupeTabs(tabs, "c");
  expect(out.tabs.map((t) => t.id)).toEqual(["a", "c"]);
  expect(out.activeId).toBe("c");
});

test("dedupeTabs keeps query tabs that differ in query or time range", () => {
  const tabs = [queryTab("a", "t1 | take 50", "1h"), queryTab("b", "t1 | take 50", "6h"), queryTab("c", "t1 | count")];
  const out = dedupeTabs(tabs, "a");
  expect(out.tabs.map((t) => t.id)).toEqual(["a", "b", "c"]);
});

test("dedupeTabs remaps activeId when the active tab is a pruned duplicate", () => {
  const tabs = [queryTab("a", "t1 | take 50"), queryTab("b", "t1 | take 50")];
  const out = dedupeTabs(tabs, "b");
  expect(out.tabs.map((t) => t.id)).toEqual(["a"]);
  expect(out.activeId).toBe("a");
});

test("dedupeTabs collapses duplicate idle discover tabs", () => {
  const tabs = [discoverTab("d1"), discoverTab("d2"), discoverTab("d3", "auth")];
  const out = dedupeTabs(tabs, "d2");
  expect(out.tabs.map((t) => t.id)).toEqual(["d1", "d3"]);
  expect(out.activeId).toBe("d1");
});

test("dedupeTabs leaves a null activeId alone", () => {
  const out = dedupeTabs([queryTab("a", "x")], null);
  expect(out.activeId).toBe(null);
  expect(out.tabs.length).toBe(1);
});

// --- findMatchingQueryTab ----------------------------------------------------

test("findMatchingQueryTab finds a query tab with the same query and range", () => {
  const tabs = [discoverTab("d"), queryTab("a", "t1 | take 50", "6h")];
  const hit = findMatchingQueryTab(tabs, "t1 | take 50", { preset: "6h" });
  expect(hit?.id).toBe("a");
});

test("findMatchingQueryTab returns undefined when query or range differ", () => {
  const tabs = [queryTab("a", "t1 | take 50", "6h")];
  expect(findMatchingQueryTab(tabs, "t1 | take 50", { preset: "1h" })).toBeUndefined();
  expect(findMatchingQueryTab(tabs, "t2 | take 50", { preset: "6h" })).toBeUndefined();
});

test("migrateWorkspace v2→v3 applies defaults to a discover tab with no pills", () => {
  const v2 = {
    state: {
      tabs: [
        { id: "x", kind: "discover", state: { scope: { kind: "all" }, search: "", pills: [] } },
      ],
      activeId: "x",
    },
  };
  const migrated = migrateWorkspace(v2, 2) as {
    state: { tabs: Array<{ state: Record<string, unknown> }> };
  };
  const st = migrated.state.tabs[0].state;
  expect(st.pills).toBeUndefined();
  expect(st.columns).toEqual([]);
  expect(st.viewMode).toBe("stream");
  expect(st.search).toBe("");
});

test("migrateWorkspace keeps a typed-but-unsubmitted v2 search", () => {
  const v2 = {
    state: {
      tabs: [
        { id: "x", kind: "discover", state: { scope: { kind: "all" }, search: "draft only", pills: [] } },
      ],
      activeId: "x",
    },
  };
  const migrated = migrateWorkspace(v2, 2) as {
    state: { tabs: Array<{ state: Record<string, unknown> }> };
  };
  expect(migrated.state.tabs[0].state.search).toBe("draft only");
});

test("migrateWorkspace is a no-op for v3", () => {
  const v3 = {
    state: {
      tabs: [
        { id: "x", kind: "discover", state: { scope: { kind: "all" }, search: "q", columns: [], viewMode: "stream" } },
      ],
      activeId: "x",
    },
  };
  const migrated = migrateWorkspace(v3, 3);
  expect(migrated).toBe(v3);
});

test("migrateWorkspace v3→v4 defaults live: false on discover tabs that lack it", () => {
  const v3 = {
    state: {
      tabs: [
        {
          id: "d",
          kind: "discover",
          state: { scope: { kind: "all" }, search: "q", columns: [], viewMode: "stream" },
        },
        {
          id: "q",
          kind: "query",
          state: { title: "q", query: "t | take 1", timeRange: { preset: "1h" }, results: { kind: "idle" }, chart: {}, submittedQuery: null },
        },
      ],
      activeId: "d",
    },
  };
  const migrated = migrateWorkspace(v3, 3) as {
    state: { tabs: Array<{ id: string; kind: string; state: Record<string, unknown> }> };
  };
  const discoverTab = migrated.state.tabs.find((t) => t.id === "d")!;
  const queryTab = migrated.state.tabs.find((t) => t.id === "q")!;
  expect(discoverTab.state.live).toBe(false);
  // query tabs are not touched
  expect(queryTab.state.live).toBeUndefined();
});

test("migrateWorkspace v2→v3 folds discover pills into the search text", () => {
  const v2 = {
    state: {
      tabs: [
        {
          id: "d",
          kind: "discover",
          state: {
            scope: { kind: "all" },
            search: "draft text",
            pills: [
              { kind: "substring", value: "auth" },
              { kind: "eq", field: "service", value: "payments" },
            ],
            timeRange: { preset: "1h" },
            visibleSources: null,
            selectedSource: null,
            results: { status: "idle", sources: new Map() },
          },
        },
      ],
      activeId: "d",
    },
  };
  const m = migrateWorkspace(v2, 2) as {
    state: { tabs: Array<{ state: { search: string; pills?: unknown; columns: string[]; viewMode: string } }> };
  };
  const st = m.state.tabs[0].state;
  expect(st.search).toBe("auth service:payments draft text");
  expect(st.pills).toBeUndefined();
  expect(st.columns).toEqual([]);
  expect(st.viewMode).toBe("stream");
});

// ── discover → query-editor export tab reuse ────────────────────────────────

test("openExportTab creates a 'from discover' tab when none exists", () => {
  const id = useWorkspace.getState().openExportTab({
    query: "events | take 50",
    timeRange: { preset: "1h" },
  });
  const t = useWorkspace.getState().tabs.find((x) => x.id === id)!;
  expect(t.kind).toBe("query");
  if (t.kind !== "query") throw new Error("type narrowing");
  expect(t.state.title).toBe("from discover");
  expect(t.state.query).toBe("events | take 50");
  expect(useWorkspace.getState().activeId).toBe(id);
});

test("openExportTab reuses a pristine export tab instead of piling up", () => {
  const ws = useWorkspace.getState();
  const first = ws.openExportTab({
    query: 'events | where timestamp >= datetime("2026-06-01T09:00:00Z") | take 500',
    timeRange: { preset: "1h" },
  });
  // Next export bakes different time bounds — the query text always differs.
  const second = useWorkspace.getState().openExportTab({
    query: 'events | where timestamp >= datetime("2026-06-07T12:00:00Z") | take 500',
    timeRange: { preset: "6h" },
  });
  expect(second).toBe(first);
  const tabs = useWorkspace.getState().tabs;
  expect(tabs.filter((t) => t.kind === "query")).toHaveLength(1);
  const t = tabs.find((x) => x.id === first)!;
  if (t.kind !== "query") throw new Error("type narrowing");
  expect(t.state.query).toContain("2026-06-07T12:00:00Z");
  expect(t.state.timeRange.preset).toBe("6h");
  expect(t.state.results).toEqual({ kind: "idle" });
  expect(t.state.submittedQuery).toBeNull();
  expect(useWorkspace.getState().activeId).toBe(first);
});

test("openExportTab never clobbers an export tab the user edited", () => {
  const ws = useWorkspace.getState();
  const first = ws.openExportTab({ query: "events | take 1", timeRange: { preset: "1h" } });
  // The user ran it, then edited the query — that's their work now.
  useWorkspace.getState().updateQuery(first, {
    submittedQuery: "events | take 1",
    query: "events | where severity == 'ERROR' | take 1",
  });
  const second = useWorkspace.getState().openExportTab({
    query: "events | take 2",
    timeRange: { preset: "1h" },
  });
  expect(second).not.toBe(first);
  const tabs = useWorkspace.getState().tabs;
  expect(tabs.filter((t) => t.kind === "query")).toHaveLength(2);
  const kept = tabs.find((x) => x.id === first)!;
  if (kept.kind !== "query") throw new Error("type narrowing");
  expect(kept.state.query).toContain("severity == 'ERROR'");
});

test("dedupeTabs collapses stale pristine exports to the newest", () => {
  const exportTab = (id: string, query: string, dirty = false): Tab => ({
    id,
    kind: "query",
    state: {
      title: "from discover",
      query,
      timeRange: { preset: "1h" },
      results: { kind: "idle" },
      chart: {},
      submittedQuery: dirty ? "something else the user ran" : null,
    },
  });
  const tabs: Tab[] = [
    exportTab("a", "events | datetime(A) | take 500"),
    exportTab("b", "events | datetime(B) | take 500", true), // edited → user work
    exportTab("c", "events | datetime(C) | take 500"),
    exportTab("d", "events | datetime(D) | take 500"),
  ];
  const { tabs: out, activeId } = dedupeTabs(tabs, "a");
  const ids = out.map((t) => t.id);
  expect(ids).toContain("d"); // newest pristine export survives
  expect(ids).toContain("b"); // dirty export is user work — kept
  expect(ids).not.toContain("a");
  expect(ids).not.toContain("c");
  expect(activeId).toBe("d"); // active pointer follows the survivor
});
