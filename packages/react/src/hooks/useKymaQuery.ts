/**
 * useKymaQuery — headless hook for running KQL/SQL queries.
 *
 * Wraps client.query.runQuery (an async generator that yields NDJSON chunks).
 * Accumulates columns + rows as chunks arrive. An AbortController is created
 * per execute() call; cancel() or component unmount both abort in-flight queries.
 *
 * Wire format: NDJSON streamed from POST /v1/query.
 * Return surface: columns (name+kind), rows (plain objects), no arrow tables
 * (the client layer returns arrow only when using the Flight path, which is
 * currently server-to-server only — the HTTP NDJSON path is the browser path).
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { Column } from "@kyma-ai/client";
import { useKymaClient } from "../provider/context";

// ── Public types ──────────────────────────────────────────────────────────────

export interface KymaQueryArgs {
  database: string;
  query: string;
  language: "kql" | "sql";
  walMs?: number;
  memBytes?: number;
}

export interface UseKymaQueryResult {
  /** Accumulated columns from the last/current query. */
  columns: Column[];
  /** Accumulated rows from the last/current query. */
  rows: Record<string, unknown>[];
  /** True while a query is streaming. */
  isRunning: boolean;
  /** Error from the last execute(), or null. */
  error: unknown;
  /**
   * Execute a query. Returns a promise that resolves when the stream is
   * complete (or rejects on error). Aborts any previously running query.
   */
  execute: (args: KymaQueryArgs) => Promise<void>;
  /** Abort the current in-flight query. No-op if nothing is running. */
  cancel: () => void;
}

// ── Implementation ────────────────────────────────────────────────────────────

export function useKymaQuery(): UseKymaQueryResult {
  const client = useKymaClient();

  const [columns, setColumns] = useState<Column[]>([]);
  const [rows, setRows] = useState<Record<string, unknown>[]>([]);
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<unknown>(null);

  // AbortController for the currently running query.
  const abortRef = useRef<AbortController | null>(null);

  // Abort on unmount.
  useEffect(() => {
    return () => {
      abortRef.current?.abort();
    };
  }, []);

  const cancel = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setIsRunning(false);
  }, []);

  const execute = useCallback(
    async (args: KymaQueryArgs): Promise<void> => {
      // Abort any in-flight query.
      abortRef.current?.abort();
      const ac = new AbortController();
      abortRef.current = ac;

      setIsRunning(true);
      setError(null);
      setColumns([]);
      setRows([]);

      try {
        for await (const chunk of client.query.runQuery({
          database: args.database,
          query: args.query,
          language: args.language,
          walMs: args.walMs,
          memBytes: args.memBytes,
          signal: ac.signal,
        })) {
          // Replace columns on first chunk (they don't change mid-stream).
          setColumns(chunk.columns);
          // Accumulate rows.
          setRows((prev) => [...prev, ...chunk.rows]);
        }
      } catch (err) {
        // Ignore abort errors — cancel() already set isRunning = false.
        if ((err as DOMException)?.name === "AbortError") {
          return;
        }
        setError(err);
        throw err;
      } finally {
        // Only clear running state if this controller is still the active one.
        if (abortRef.current === ac) {
          setIsRunning(false);
          abortRef.current = null;
        }
      }
    },
    [client],
  );

  return { columns, rows, isRunning, error, execute, cancel };
}
