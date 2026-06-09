/**
 * useKymaDiscover — headless hook for streaming Discover search.
 *
 * Wraps client.discover.searchDiscover (async generator over Frame objects).
 * Frames are accumulated as they arrive; `isStreaming` is true while the
 * generator is running. An AbortController per search() call allows in-flight
 * cancellation; unmount aborts automatically.
 *
 * Wire format: NDJSON-over-HTTP from POST /v1/explore/search.
 * Frame types: plan | source_progress | rows | histogram | source_done |
 *              error | done | live | heartbeat
 */

import { useCallback, useEffect, useRef, useState } from "react";
import type { Frame, SearchRequest } from "@kyma-ai/client";
import { useKymaClient } from "../provider/context";

// ── Public types ──────────────────────────────────────────────────────────────

export interface UseKymaDiscoverResult {
  /** All frames received from the current / last search stream. */
  frames: Frame[];
  /** True while the search generator is running. */
  isStreaming: boolean;
  /** Error from the last search(), or null. */
  error: unknown;
  /**
   * Start a new search. Aborts any running search first, then resets frames.
   * Returns a promise that resolves when the stream is complete.
   */
  search: (req: SearchRequest) => Promise<void>;
  /** Abort the current in-flight search stream. No-op if not streaming. */
  abort: () => void;
}

// ── Implementation ────────────────────────────────────────────────────────────

export function useKymaDiscover(): UseKymaDiscoverResult {
  const client = useKymaClient();

  const [frames, setFrames] = useState<Frame[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<unknown>(null);

  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    return () => {
      abortRef.current?.abort();
    };
  }, []);

  const abort = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setIsStreaming(false);
  }, []);

  const search = useCallback(
    async (req: SearchRequest): Promise<void> => {
      // Abort any in-flight search.
      abortRef.current?.abort();
      const ac = new AbortController();
      abortRef.current = ac;

      setIsStreaming(true);
      setError(null);
      setFrames([]);

      try {
        for await (const frame of client.discover.searchDiscover(req, ac.signal)) {
          setFrames((prev) => [...prev, frame]);
        }
      } catch (err) {
        // Ignore abort.
        if ((err as DOMException)?.name === "AbortError") {
          return;
        }
        // DiscoverError or plain Error from the client layer.
        setError(err);
        throw err;
      } finally {
        if (abortRef.current === ac) {
          setIsStreaming(false);
          abortRef.current = null;
        }
      }
    },
    [client],
  );

  return { frames, isStreaming, error, search, abort };
}
