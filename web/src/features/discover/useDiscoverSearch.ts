import { useCallback, useEffect, useRef, useState } from "react";
import { searchDiscover, type SearchRequest } from "../../sdk/discover";
import { LiveSession, type LiveStatus } from "../../sdk/discover-live";
import { useSession } from "../../sdk/session";
import { applyFrame } from "./discover-store";
import type { DiscoverResultsState, Scope } from "./types";
import type { TimeRange } from "../tabs/workspace-store";

type Args = {
  search: string;
  scope: Scope;
  timeRange: TimeRange;
  perSourceLimit?: number;
  enabled: boolean;
  live?: boolean;
};

const PRESET_MINUTES: Record<string, number> = {
  "5m": 5, "15m": 15, "1h": 60, "6h": 360,
  "24h": 1440, "7d": 10080, "30d": 43200,
};

export function resolveTimeRange(t: TimeRange): { from: string; to: string } | null {
  if (t.preset === "custom" && t.from && t.to) return { from: t.from, to: t.to };
  const minutes = PRESET_MINUTES[t.preset];
  if (minutes == null) return null;
  const now = Date.now();
  return {
    from: new Date(now - minutes * 60_000).toISOString(),
    to: new Date(now).toISOString(),
  };
}

const emptyResults = (): DiscoverResultsState => ({
  status: "idle",
  sources: new Map(),
});

export function useDiscoverSearch(args: Args) {
  const [results, setResults] = useState<DiscoverResultsState>(emptyResults);
  const [liveStatus, setLiveStatus] = useState<LiveStatus | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const liveSessionRef = useRef<LiveSession | null>(null);

  const { endpoint, token } = useSession();

  const argsKey = JSON.stringify({
    scope: args.scope,
    search: args.search,
    timeRange: args.timeRange,
    perSourceLimit: args.perSourceLimit ?? 500,
  });

  // ── one-shot POST path ────────────────────────────────────────────────────
  const run = useCallback(async () => {
    abortRef.current?.abort();
    const ac = new AbortController();
    abortRef.current = ac;

    let acc: DiscoverResultsState = {
      status: "running",
      sources: new Map(),
      startedAt: Date.now(),
    };
    setResults(acc);

    const req: SearchRequest = {
      query: args.search,
      scope: args.scope,
      time_range: resolveTimeRange(args.timeRange),
      per_source_limit: args.perSourceLimit ?? 500,
    };

    try {
      for await (const frame of searchDiscover(req, ac.signal)) {
        if (ac.signal.aborted) return;
        acc = applyFrame(acc, frame);
        setResults(acc);
      }
    } catch (e: unknown) {
      if ((e as Error)?.name === "AbortError") return;
      const err = e as { code?: string; message?: string };
      acc = {
        ...acc,
        status: "error",
        topError: {
          code: err.code ?? "fetch_error",
          message: err.message ?? String(e),
        },
      };
      setResults(acc);
    }
  }, [argsKey]);

  // ── live WebSocket path ───────────────────────────────────────────────────
  const startLive = useCallback(() => {
    // Tear down any previous session or abort
    abortRef.current?.abort();
    if (liveSessionRef.current) {
      liveSessionRef.current.close();
      liveSessionRef.current = null;
    }

    let acc: DiscoverResultsState = {
      status: "running",
      sources: new Map(),
      startedAt: Date.now(),
    };
    setResults(acc);

    const body = {
      query: args.search,
      scope: args.scope,
      time_range: resolveTimeRange(args.timeRange),
      per_source_limit: args.perSourceLimit ?? 500,
    };

    const session = new LiveSession({
      endpoint,
      token,
      body,
      onFrame: (frame) => {
        acc = applyFrame(acc, frame);
        setResults(acc);
      },
      onStatus: (s) => {
        setLiveStatus(s);
      },
    });
    liveSessionRef.current = session;
  }, [endpoint, token, argsKey]);

  // ── effect: run or startLive based on the `live` flag ────────────────────
  useEffect(() => {
    if (!args.enabled) return;

    if (args.live) {
      startLive();
    } else {
      void run();
    }

    return () => {
      abortRef.current?.abort();
      if (liveSessionRef.current) {
        liveSessionRef.current.close();
        liveSessionRef.current = null;
      }
      setLiveStatus(null);
    };
  }, [args.enabled, args.live, args.live ? startLive : run]);

  // When args change while live (e.g. query edit + Enter), update the session
  // in-place rather than reconnecting.
  useEffect(() => {
    if (!args.enabled || !args.live || !liveSessionRef.current) return;
    liveSessionRef.current.update({
      query: args.search,
      scope: args.scope,
      time_range: resolveTimeRange(args.timeRange),
      per_source_limit: args.perSourceLimit ?? 500,
    });
  }, [argsKey]);

  const cancel = useCallback(() => {
    abortRef.current?.abort();
    if (liveSessionRef.current) {
      liveSessionRef.current.close();
      liveSessionRef.current = null;
    }
    setLiveStatus(null);
  }, []);

  return { results, rerun: run, cancel, liveStatus };
}
