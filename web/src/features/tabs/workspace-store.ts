import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import type { DiscoverTabState } from "../discover/discover-store";
import { initialDiscoverTabState } from "../discover/discover-store";
import { serializePills } from "../discover/discoverGrammar";
import type { Pill } from "../discover/types";

// Defensive storage: tests run under jsdom but the persist middleware may grab
// `localStorage` before jsdom finishes wiring it. Wrap with a no-op fallback so
// the store doesn't throw on `set` when storage is unavailable.
const noopStorage = {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
};

export type TimeRangePreset =
  | "5m" | "15m" | "1h" | "6h" | "24h" | "7d" | "30d" | "custom";
export type TimeRange = { preset: TimeRangePreset; from?: string; to?: string };

export type ResultsState =
  | { kind: "idle" }
  | { kind: "running"; startedAt: number }
  | { kind: "ok"; rowCount: number; durationMs: number; finishedAt: number }
  | { kind: "error"; message: string; requestId?: string; code?: string };

export type ChartConfig = {
  override?: { type: "line" | "bar" | "scatter" | "stat"; x?: string; y?: string };
};

export type QueryTabState = {
  title: string;
  query: string;
  timeRange: TimeRange;
  results: ResultsState;
  chart: ChartConfig;
  submittedQuery: string | null;
};

export type Tab =
  | { id: string; kind: "query"; state: QueryTabState }
  | { id: string; kind: "discover"; state: DiscoverTabState };

export type QueryTab = Extract<Tab, { kind: "query" }>;
export type DiscoverTab = Extract<Tab, { kind: "discover" }>;

export function isQueryTabDirty(t: QueryTab): boolean {
  return t.state.submittedQuery !== null && t.state.submittedQuery !== t.state.query;
}

// Compatibility wrapper used by TabBar — only query tabs can be "dirty".
export function isTabDirty(t: Tab): boolean {
  return t.kind === "query" && isQueryTabDirty(t);
}

const DEFAULT_RANGE: TimeRange = { preset: "1h" };

// Exported so tests can verify the v1→v2/v3 transformation as a pure function
// without touching real localStorage / persist middleware machinery.
export function migrateWorkspace(persisted: unknown, fromVersion: number): unknown {
  const p = persisted as { state?: { tabs?: unknown[] } } | null;
  if (!p?.state?.tabs) return persisted;
  if (fromVersion < 2) {
    p.state.tabs = p.state.tabs.map((oldRaw) => {
      const old = oldRaw as {
        id?: string; title?: string; query?: string;
        timeRange?: TimeRange; results?: ResultsState;
        chart?: ChartConfig; submittedQuery?: string | null;
      };
      return {
        id: old.id ?? crypto.randomUUID(),
        kind: "query" as const,
        state: {
          title: old.title ?? "query",
          query: old.query ?? "",
          timeRange: old.timeRange ?? DEFAULT_RANGE,
          results: old.results ?? { kind: "idle" },
          chart: old.chart ?? {},
          submittedQuery: old.submittedQuery ?? null,
        },
      };
    });
  }
  if (fromVersion < 3) {
    p.state.tabs = (p.state.tabs as Array<Record<string, unknown>>).map((t) => {
      if (t.kind !== "discover") return t;
      const st = t.state as {
        pills?: unknown[];
        search?: string;
        columns?: string[];
      } & Record<string, unknown>;
      const pillText = Array.isArray(st.pills) && st.pills.length > 0
        ? serializePills(st.pills as Pill[])
        : "";
      const search = [pillText, st.search ?? ""].filter(Boolean).join(" ").trim();
      const { pills: _pills, ...rest } = st;
      return {
        ...t,
        state: { ...rest, search, columns: st.columns ?? [], viewMode: "stream" },
      };
    });
  }
  return persisted;
}

function sameTimeRange(a: TimeRange, b: TimeRange): boolean {
  return a.preset === b.preset && (a.from ?? "") === (b.from ?? "") && (a.to ?? "") === (b.to ?? "");
}

// Stable identity for "this tab shows the same thing as that one". Titles and
// transient results are deliberately excluded.
function tabIdentity(t: Tab): string {
  if (t.kind === "query") {
    const { query, timeRange } = t.state;
    return `q|${query}|${timeRange.preset}|${timeRange.from ?? ""}|${timeRange.to ?? ""}`;
  }
  const { scope, search } = t.state;
  return `d|${search}|${JSON.stringify(scope)}`;
}

// Collapse tabs whose content is identical, keeping the first occurrence.
// Heals workspaces that accumulated duplicates (every reload of a `?q=` deep
// link used to spawn a fresh tab). Remaps `activeId` onto the surviving
// duplicate so the user stays "on" the same content.
export function dedupeTabs(
  tabs: Tab[],
  activeId: string | null,
): { tabs: Tab[]; activeId: string | null } {
  const keptByIdentity = new Map<string, Tab>();
  const kept: Tab[] = [];
  for (const t of tabs) {
    const key = tabIdentity(t);
    if (!keptByIdentity.has(key)) {
      keptByIdentity.set(key, t);
      kept.push(t);
    }
  }
  if (kept.length === tabs.length) return { tabs, activeId };
  let nextActive = activeId;
  if (activeId && !kept.some((t) => t.id === activeId)) {
    const pruned = tabs.find((t) => t.id === activeId);
    nextActive = pruned ? keptByIdentity.get(tabIdentity(pruned))?.id ?? null : null;
  }
  return { tabs: kept, activeId: nextActive };
}

// Find an existing query tab showing exactly this query + time range — used
// by the `?q=` deep-link bootstrap to reuse instead of spawning a new tab.
export function findMatchingQueryTab(
  tabs: Tab[],
  query: string,
  timeRange: TimeRange,
): QueryTab | undefined {
  return tabs.find(
    (t): t is QueryTab =>
      t.kind === "query" && t.state.query === query && sameTimeRange(t.state.timeRange, timeRange),
  );
}

function defaultQueryState(idx: number): QueryTabState {
  return {
    title: `query ${idx}`,
    query: "",
    timeRange: DEFAULT_RANGE,
    results: { kind: "idle" },
    chart: {},
    submittedQuery: null,
  };
}

type NewTabSeed =
  | { kind?: "query"; state?: Partial<QueryTabState> }
  | { kind: "discover"; state?: Partial<DiscoverTabState> };

type Store = {
  tabs: Tab[];
  activeId: string | null;
  newTab: (seed?: NewTabSeed) => string;
  closeTab: (id: string) => void;
  setActive: (id: string) => void;
  updateQuery: (id: string, patch: Partial<QueryTabState>) => void;
  updateDiscover: (id: string, next: DiscoverTabState) => void;
  patchDiscover: (id: string, patch: Partial<DiscoverTabState>) => void;
  resetAll: () => void;
};

export const useWorkspace = create<Store>()(
  persist(
    (set, get) => ({
      tabs: [],
      activeId: null,
      newTab: (seed) => {
        const id = crypto.randomUUID();
        const idx = get().tabs.length + 1;
        const kind = seed?.kind ?? "discover";
        const tab: Tab =
          kind === "query"
            ? {
                id,
                kind: "query",
                state: {
                  ...defaultQueryState(idx),
                  ...((seed?.state ?? {}) as Partial<QueryTabState>),
                },
              }
            : {
                id,
                kind: "discover",
                state: {
                  ...initialDiscoverTabState(),
                  ...((seed?.state ?? {}) as Partial<DiscoverTabState>),
                },
              };
        set({ tabs: [...get().tabs, tab], activeId: id });
        return id;
      },
      closeTab: (id) => {
        const tabs = get().tabs.filter((t) => t.id !== id);
        const activeId = get().activeId === id ? tabs.at(-1)?.id ?? null : get().activeId;
        set({ tabs, activeId });
      },
      setActive: (id) => set({ activeId: id }),
      updateQuery: (id, patch) =>
        set({
          tabs: get().tabs.map((t) =>
            t.id === id && t.kind === "query"
              ? { ...t, state: { ...t.state, ...patch } }
              : t,
          ),
        }),
      updateDiscover: (id, next) =>
        set({
          tabs: get().tabs.map((t) =>
            t.id === id && t.kind === "discover"
              ? { ...t, state: next }
              : t,
          ),
        }),
      patchDiscover: (id, patch) =>
        set({
          tabs: get().tabs.map((t) =>
            t.id === id && t.kind === "discover"
              ? { ...t, state: { ...t.state, ...patch } }
              : t,
          ),
        }),
      resetAll: () => set({ tabs: [], activeId: null }),
    }),
    {
      name: "kyma.workspace",
      version: 3,
      storage: createJSONStorage(() =>
        typeof window !== "undefined" && window.localStorage
          ? window.localStorage
          : noopStorage,
      ),
      migrate: (persisted, fromVersion) => migrateWorkspace(persisted, fromVersion),
      // Discover results.sources is a Map and the rows can be arbitrarily large.
      // Strip transient results on persist; rehydrate to idle.
      partialize: (s) => ({
        tabs: s.tabs.map((t) =>
          t.kind === "discover"
            ? {
                ...t,
                state: {
                  ...t.state,
                  results: { status: "idle" as const, sources: new Map() },
                },
              }
            : t,
        ),
        activeId: s.activeId,
      }),
      onRehydrateStorage: () => (s) => {
        if (!s) return;
        for (const t of s.tabs) {
          if (t.kind === "discover") {
            t.state.results = { status: "idle", sources: new Map() };
          }
        }
        // Heal duplicate tabs accumulated by older deep-link bootstrapping.
        const deduped = dedupeTabs(s.tabs, s.activeId);
        s.tabs = deduped.tabs;
        s.activeId = deduped.activeId;
      },
    },
  ),
);
