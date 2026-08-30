// Per-instance Discover state. Held as a zustand/vanilla store so two mounted
// PensieveDiscover components never share results state.
// (Mirrors the web/src/features/discover/discover-store.ts reducer but uses the
// vanilla factory pattern matching graph-store.ts.)

import { createStore } from "zustand/vanilla";
import { useStore } from "zustand";
import { createContext, useContext } from "react";
import type {
  DiscoverResultsState,
  Frame,
  Scope,
  SourceKey,
  SourceState,
} from "./types";
import type { TimeRange } from "../query/time-range/time-range-types";

// ── Frame reducer ─────────────────────────────────────────────────────────────

export function applyFrame(
  state: DiscoverResultsState,
  frame: Frame,
): DiscoverResultsState {
  const next: DiscoverResultsState = {
    ...state,
    sources: new Map(state.sources),
  };
  switch (frame.type) {
    case "plan": {
      next.status = "running";
      next.startedAt = next.startedAt ?? Date.now();
      next.sources = new Map(
        frame.sources.map((s): [SourceKey, SourceState] => [
          s.source,
          {
            source: s.source,
            hasTimestamp: s.has_timestamp,
            timestampColumn: s.timestamp_column ?? null,
            progress: "pending",
            rows: [],
            total: 0,
            capped: false,
            droppedClauses: [],
          },
        ]),
      );
      return next;
    }
    case "source_progress": {
      const s = next.sources.get(frame.source);
      if (s) next.sources.set(frame.source, { ...s, progress: "running" });
      return next;
    }
    case "rows": {
      const s = next.sources.get(frame.source);
      if (s)
        next.sources.set(frame.source, {
          ...s,
          rows: [...s.rows, ...frame.rows],
        });
      return next;
    }
    case "histogram": {
      const s = next.sources.get(frame.source);
      if (s)
        next.sources.set(frame.source, {
          ...s,
          histogram: frame.buckets,
        });
      return next;
    }
    case "source_done": {
      const s = next.sources.get(frame.source);
      if (s)
        next.sources.set(frame.source, {
          ...s,
          progress: "done",
          total: frame.total,
          capped: frame.capped,
          droppedClauses: frame.dropped_clauses,
        });
      return next;
    }
    case "error": {
      if (frame.source) {
        const s = next.sources.get(frame.source);
        if (s)
          next.sources.set(frame.source, {
            ...s,
            progress: "error",
            error: { code: frame.code, message: frame.message },
          });
      } else {
        next.topError = { code: frame.code, message: frame.message };
      }
      return next;
    }
    case "done": {
      next.status = next.topError ? "error" : "done";
      next.finishedAt = Date.now();
      return next;
    }
    case "live": {
      next.status = "live";
      return next;
    }
    case "heartbeat": {
      return next;
    }
  }
}

// ── Per-instance store shape ──────────────────────────────────────────────────

export type DiscoverStoreState = {
  search: string;
  scope: Scope;
  timeRange: TimeRange;
  visibleSources: SourceKey[] | null;
  selectedSource: SourceKey | null;
  columns: string[];
  viewMode: "stream" | { table: SourceKey };
  submitted: string;
  results: DiscoverResultsState;

  setSearch(v: string): void;
  setScope(s: Scope): void;
  setTimeRange(tr: TimeRange): void;
  toggleVisible(src: SourceKey, allKeys: SourceKey[]): void;
  setVisible(v: SourceKey[] | null): void;
  setSelected(src: SourceKey | null): void;
  toggleColumn(f: string): void;
  setViewMode(m: "stream" | { table: SourceKey }): void;
  submit(): void;
  setResults(r: DiscoverResultsState): void;
};

// ── Factory ───────────────────────────────────────────────────────────────────

export type DiscoverStore = ReturnType<typeof createDiscoverStore>;

export function createDiscoverStore(initial?: {
  search?: string;
  scope?: Scope;
  timeRange?: TimeRange;
}) {
  const init = {
    search: initial?.search ?? "",
    scope: (initial?.scope ?? { kind: "all" }) as Scope,
    // Default to "all time" so a fresh Discover load shows data whenever any
    // exists (data is often older than a 1h window); the user then narrows.
    timeRange: (initial?.timeRange ?? { preset: "none" }) as TimeRange,
    visibleSources: null as SourceKey[] | null,
    selectedSource: null as SourceKey | null,
    columns: [] as string[],
    viewMode: "stream" as "stream" | { table: SourceKey },
    submitted: initial?.search ?? "",
    results: { status: "idle", sources: new Map() } as DiscoverResultsState,
  };

  return createStore<DiscoverStoreState>()((set) => ({
    ...init,
    setSearch: (v) => set({ search: v }),
    setScope: (s) => set({ scope: s }),
    setTimeRange: (tr) => set({ timeRange: tr }),
    toggleVisible: (src, allKeys) =>
      set((st) => {
        const cur = st.visibleSources ?? allKeys;
        const next = cur.includes(src) ? cur.filter((s) => s !== src) : [...cur, src];
        return { visibleSources: next };
      }),
    setVisible: (v) => set({ visibleSources: v }),
    setSelected: (src) => set({ selectedSource: src }),
    toggleColumn: (f) =>
      set((st) => ({
        columns: st.columns.includes(f)
          ? st.columns.filter((c) => c !== f)
          : [...st.columns, f],
      })),
    setViewMode: (m) => set({ viewMode: m }),
    submit: () => set((st) => ({ submitted: st.search })),
    setResults: (r) => set({ results: r }),
  }));
}

// ── Context ───────────────────────────────────────────────────────────────────

export const DiscoverStoreContext = createContext<DiscoverStore | null>(null);

export function useDiscoverStore<T>(selector: (s: DiscoverStoreState) => T): T {
  const store = useContext(DiscoverStoreContext);
  if (!store) throw new Error("useDiscoverStore must be used inside <PensieveDiscover>");
  return useStore(store, selector);
}
