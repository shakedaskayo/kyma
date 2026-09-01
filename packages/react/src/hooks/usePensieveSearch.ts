/**
 * usePensieveSearch — headless hook for the unified hybrid search (`/v1/search`).
 *
 * Lexical + vector (RRF-fused) ranked results across the sources in scope. Each
 * hit carries `db.table` provenance + the row (data mode) or rich metadata
 * (memory/graph modes), so the caller can drill down with a follow-up KQL/SQL
 * query or navigate to the graph.
 */
import { useCallback, useRef, useState } from "react";
import type { HybridSearchHit, HybridSearchRequest } from "@pensieve-ai/client";
import { usePensieveClient } from "../provider/context";

export interface UsePensieveSearchResult {
  hits: HybridSearchHit[];
  sourcesSearched: number;
  elapsedMs: number;
  /** Echo of the mode the server used (absent when mode was "data"). */
  mode: string | undefined;
  /** True once a search has been run (distinguishes "idle" from "0 hits"). */
  ran: boolean;
  isRunning: boolean;
  error: unknown;
  /** Run a search. Supersedes any in-flight call. */
  run: (req: HybridSearchRequest) => Promise<void>;
}

export function usePensieveSearch(): UsePensieveSearchResult {
  const client = usePensieveClient();
  const [hits, setHits] = useState<HybridSearchHit[]>([]);
  const [sourcesSearched, setSourcesSearched] = useState(0);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [mode, setMode] = useState<string | undefined>(undefined);
  const [ran, setRan] = useState(false);
  const [isRunning, setIsRunning] = useState(false);
  const [error, setError] = useState<unknown>(null);
  // Monotonic token so a slow earlier call can't overwrite a newer one.
  const seq = useRef(0);

  const run = useCallback(
    async (req: HybridSearchRequest) => {
      const mine = ++seq.current;
      setIsRunning(true);
      setError(null);
      try {
        const r = await client.search.search(req);
        if (mine !== seq.current) return;
        setHits(r.hits);
        setSourcesSearched(r.sources_searched);
        setElapsedMs(r.elapsed_ms);
        setMode(r.mode);
        setRan(true);
      } catch (e) {
        if (mine !== seq.current) return;
        setError(e);
        setHits([]);
        setRan(true);
      } finally {
        if (mine === seq.current) setIsRunning(false);
      }
    },
    [client],
  );

  return { hits, sourcesSearched, elapsedMs, mode, ran, isRunning, error, run };
}
