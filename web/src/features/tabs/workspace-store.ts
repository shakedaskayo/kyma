import { create } from "zustand";
import { persist } from "zustand/middleware";

export type TimeRangePreset = "5m" | "15m" | "1h" | "6h" | "24h" | "7d" | "30d" | "custom";
export type TimeRange = { preset: TimeRangePreset; from?: string; to?: string };

export type ResultsState =
  | { kind: "idle" }
  | { kind: "running"; startedAt: number }
  | { kind: "ok"; rowCount: number; durationMs: number; finishedAt: number }
  | { kind: "error"; message: string; requestId?: string; code?: string };

export type ChartConfig = {
  override?: { type: "line" | "bar" | "scatter" | "stat"; x?: string; y?: string };
};

export type Tab = {
  id: string;
  title: string;
  query: string;
  timeRange: TimeRange;
  results: ResultsState;
  chart: ChartConfig;
  submittedQuery: string | null;
};

export function isTabDirty(tab: Tab): boolean {
  return tab.submittedQuery !== null && tab.submittedQuery !== tab.query;
}

type Store = {
  tabs: Tab[];
  activeId: string | null;
  newTab: (seed?: Partial<Tab>) => string;
  closeTab: (id: string) => void;
  setActive: (id: string) => void;
  setQuery: (id: string, q: string) => void;
  setTimeRange: (id: string, r: TimeRange) => void;
  setResults: (id: string, r: ResultsState) => void;
  setChart: (id: string, c: ChartConfig) => void;
  markSubmitted: (id: string, query: string) => void;
  resetAll: () => void;
};

const DEFAULT_RANGE: TimeRange = { preset: "1h" };

export const useWorkspace = create<Store>()(
  persist(
    (set, get) => ({
      tabs: [],
      activeId: null,
      newTab: (seed) => {
        const id = crypto.randomUUID();
        const tab: Tab = {
          id, title: `query ${get().tabs.length + 1}`, query: "", timeRange: DEFAULT_RANGE,
          results: { kind: "idle" }, chart: {}, submittedQuery: null, ...seed,
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
      setQuery: (id, query) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, query } : t)),
      }),
      setTimeRange: (id, timeRange) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, timeRange } : t)),
      }),
      setResults: (id, results) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, results } : t)),
      }),
      setChart: (id, chart) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, chart } : t)),
      }),
      markSubmitted: (id, query) => set({
        tabs: get().tabs.map((t) => (t.id === id ? { ...t, submittedQuery: query } : t)),
      }),
      resetAll: () => set({ tabs: [], activeId: null }),
    }),
    { name: "kyma.workspace" },
  ),
);
