import { useCallback, useRef, useState } from "react";
import { runQuery, type Column } from "@/sdk/query";
import { useSession } from "@/sdk/session";
import { prependTimeFilter } from "@/features/time-range/time-range";
import type { TimeRange } from "@/features/tabs/workspace-store";

export type PanelResult =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; columns: Column[]; rows: Record<string, unknown>[] }
  | { kind: "error"; message: string };

/** Runs a single panel query with time-range prepended. */
export function usePanelQuery() {
  const [result, setResult] = useState<PanelResult>({ kind: "idle" });
  const abortRef = useRef<AbortController | null>(null);

  const run = useCallback(
    async (query: string, databaseName: string | null, timeRange: TimeRange) => {
      const { endpoint, token, database } = useSession.getState();
      if (!endpoint || !token) return;

      abortRef.current?.abort();
      const ctl = new AbortController();
      abortRef.current = ctl;

      setResult({ kind: "loading" });

      const db = databaseName || database;
      const finalQuery = prependTimeFilter(query, timeRange);

      const acc: { columns: Column[]; rows: Record<string, unknown>[] } = { columns: [], rows: [] };
      try {
        for await (const chunk of runQuery({
          endpoint,
          token,
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
    [],
  );

  const cancel = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setResult({ kind: "idle" });
  }, []);

  return { result, run, cancel };
}
