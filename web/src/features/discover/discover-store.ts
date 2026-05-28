// Per-tab Discover state. Held inside the workspace store's discriminated
// `discover` tab kind (added in Phase 6). This module is the *shape* + the
// pure frame reducer so the workspace store can hold and persist it.

import type {
  DiscoverResultsState,
  Frame,
  Pill,
  Scope,
  SourceKey,
  SourceState,
} from "./types";
import type { TimeRange } from "../tabs/workspace-store";

export type DiscoverTabState = {
  scope: Scope;
  search: string;
  pills: Pill[];
  timeRange: TimeRange;
  visibleSources: SourceKey[] | null; // null = all in plan
  selectedSource: SourceKey | null;   // drives the Fields rail
  results: DiscoverResultsState;
};

export const initialDiscoverTabState = (): DiscoverTabState => ({
  scope: { kind: "all" },
  search: "",
  pills: [],
  timeRange: { preset: "1h" },
  visibleSources: null,
  selectedSource: null,
  results: { status: "idle", sources: new Map() },
});

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
  }
}
