import { useCallback, useRef } from "react";
import { toast } from "sonner";
import { runQuery, type Column } from "@/sdk/query";
import { useSession } from "@/sdk/session";
import { useWorkspace, type Tab } from "@/features/tabs/workspace-store";
import { prependTimeFilter } from "@/features/time-range/time-range";

export type TabResult = {
  columns: Column[];
  rows: Record<string, unknown>[];
  chartPoints: Record<string, unknown>[];
};

export function useRunQuery() {
  const aborters = useRef(new Map<string, AbortController>());

  const run = useCallback(async (tab: Tab, onBatch: (r: TabResult) => void) => {
    const { endpoint, token, database } = useSession.getState();
    const workspace = useWorkspace.getState();
    if (!endpoint || !token) { toast.error("Configure server + token in Settings"); return; }

    // Cancel any in-flight run for this tab.
    aborters.current.get(tab.id)?.abort();
    const ctl = new AbortController();
    aborters.current.set(tab.id, ctl);

    const finalQuery = prependTimeFilter(tab.query, tab.timeRange);
    const startedAt = performance.now();
    workspace.markSubmitted(tab.id, tab.query);
    workspace.setResults(tab.id, { kind: "running", startedAt: Date.now() });

    const acc: TabResult = { columns: [], rows: [], chartPoints: [] };
    try {
      for await (const chunk of runQuery({
        endpoint, token, database, query: finalQuery, language: "kql", signal: ctl.signal,
      })) {
        if (acc.columns.length === 0) acc.columns = chunk.columns;
        acc.rows.push(...chunk.rows);
        acc.chartPoints.push(...chunk.rows);
        onBatch({
          columns: acc.columns,
          rows: acc.rows.slice(),
          chartPoints: acc.chartPoints.slice(),
        });
      }
      const durationMs = performance.now() - startedAt;
      workspace.setResults(tab.id, {
        kind: "ok", rowCount: acc.rows.length,
        durationMs, finishedAt: Date.now(),
      });
    } catch (e) {
      if ((e as Error).name === "AbortError") {
        workspace.setResults(tab.id, { kind: "idle" });
        return;
      }
      const msg = (e as Error).message || "query failed";
      toast.error(msg);
      workspace.setResults(tab.id, { kind: "error", message: msg });
    }
  }, []);

  const cancel = useCallback((tabId: string) => {
    aborters.current.get(tabId)?.abort();
    aborters.current.delete(tabId);
  }, []);

  return { run, cancel };
}
