/**
 * usePanelQuery — per-panel query runner for dashboard panels.
 *
 * Wraps useKymaQuery (per-instance, no shared store) with:
 *   - time-range injection via prependTimeFilter
 *   - panel-level abort on re-run
 *
 * Decoupled from web's useDashboardQuery (which called useSession.getState()
 * from Zustand — here we use the client from KymaProvider context instead).
 */

import { useCallback, useRef, useState } from "react";
import type { Column } from "@kyma-ai/client";
import { useKymaClient } from "../provider/context";
import { prependTimeFilter } from "../query/time-range/time-range";
import type { TimeRange } from "../query/time-range/time-range-types";

export type PanelResult =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; columns: Column[]; rows: Record<string, unknown>[] }
  | { kind: "error"; message: string };

/** Runs a single dashboard-panel query with time-range prepended. */
export function usePanelQuery() {
  const client = useKymaClient();
  const [result, setResult] = useState<PanelResult>({ kind: "idle" });
  const abortRef = useRef<AbortController | null>(null);

  const run = useCallback(
    async (query: string, databaseName: string | null, timeRange: TimeRange) => {
      abortRef.current?.abort();
      const ctl = new AbortController();
      abortRef.current = ctl;

      setResult({ kind: "loading" });

      const db = databaseName ?? client.transport.database ?? "default";
      const finalQuery = prependTimeFilter(query, timeRange);

      const acc: { columns: Column[]; rows: Record<string, unknown>[] } = { columns: [], rows: [] };
      try {
        for await (const chunk of client.query.runQuery({
          database: db,
          query: finalQuery,
          language: "kql",
          signal: ctl.signal,
        })) {
          if (acc.columns.length === 0) acc.columns = chunk.columns;
          acc.rows.push(...chunk.rows);
          setResult({ kind: "ok", columns: [...acc.columns], rows: [...acc.rows] });
        }
        if (acc.rows.length === 0 && acc.columns.length === 0) {
          setResult({ kind: "ok", columns: [], rows: [] });
        }
      } catch (e) {
        if ((e as Error).name === "AbortError") return;
        setResult({ kind: "error", message: (e as Error).message });
      }
    },
    [client],
  );

  const cancel = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setResult({ kind: "idle" });
  }, []);

  return { result, run, cancel };
}
