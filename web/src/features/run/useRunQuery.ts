import { useCallback, useRef } from "react";
import { toast } from "sonner";
import { runQuery, type Column } from "@/sdk/query";
import { useSession } from "@/sdk/session";
import { useWorkspace, type Tab } from "@/features/tabs/workspace-store";
import { prependTimeFilter } from "@/features/time-range/time-range";
import { extractLeadingTable } from "@/features/search/parseSearch";

export type TabResult = {
  columns: Column[];
  rows: Record<string, unknown>[];
  chartPoints: Record<string, unknown>[];
};

/**
 * @param timestampTables Tables that actually have a `timestamp` column. The
 * time-range filter is only injected for those — code/graph tables without a
 * timestamp would otherwise fail every query with "column not found". When
 * omitted, the filter is injected (legacy behaviour).
 */
export function useRunQuery(timestampTables?: Set<string>) {
  const aborters = useRef(new Map<string, AbortController>());

  const run = useCallback(async (tab: Tab, onBatch: (r: TabResult) => void) => {
    const { endpoint, token, database } = useSession.getState();
    const workspace = useWorkspace.getState();
    if (!endpoint || !token) { toast.error("Configure server + token in Settings"); return; }

    // Cancel any in-flight run for this tab.
    aborters.current.get(tab.id)?.abort();
    const ctl = new AbortController();
    aborters.current.set(tab.id, ctl);

    // Only add the time filter when the leading table has a `timestamp` column.
    const lead = extractLeadingTable(tab.query);
    const hasTime = lead != null && (timestampTables?.has(lead) ?? true);
    const finalQuery = hasTime ? prependTimeFilter(tab.query, tab.timeRange) : tab.query;
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
  }, [timestampTables]);

  const cancel = useCallback((tabId: string) => {
    aborters.current.get(tabId)?.abort();
    aborters.current.delete(tabId);
  }, []);

  return { run, cancel };
}
