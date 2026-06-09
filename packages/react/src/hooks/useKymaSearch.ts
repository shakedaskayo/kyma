/**
 * useKymaSearch — headless hook for the unified hybrid search (`/v1/search`).
 *
 * Lexical + vector (RRF-fused) ranked results across the sources in scope. Each
 * hit carries `db.table` provenance + the row, so the caller can drill down with
 * a follow-up KQL/SQL query.
 */
import { useCallback, useRef, useState } from "react";
import type { HybridSearchHit, HybridSearchRequest } from "@kyma-ai/client";
import { useKymaClient } from "../provider/context";

export interface UseKymaSearchResult {
  hits: HybridSearchHit[];
  isRunning: boolean;
  error: unknown;
  /** Run a search. Supersedes any in-flight call. */
  run: (req: HybridSearchRequest) => Promise<void>;
}

export function useKymaSearch(): UseKymaSearchResult {
  const client = useKymaClient();
  const [hits, setHits] = useState<HybridSearchHit[]>([]);
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
      } catch (e) {
        if (mine !== seq.current) return;
        setError(e);
        setHits([]);
      } finally {
        if (mine === seq.current) setIsRunning(false);
      }
    },
    [client],
  );

  return { hits, isRunning, error, run };
}
