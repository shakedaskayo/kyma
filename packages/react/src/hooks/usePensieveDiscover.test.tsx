/**
 * useKymaDiscover tests.
 *
 * searchDiscover is an async generator over NDJSON frames (Frame union type).
 * We stub fetch to return a Response with a ReadableStream body.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, cleanup, waitFor, act } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { KymaProvider } from "../provider/KymaProvider";
import { useKymaDiscover } from "./useKymaDiscover";
import type { Frame } from "@kyma-ai/client";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
}

function wrapper(qc: QueryClient) {
  return ({ children }: { children: React.ReactNode }) => (
    <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }} queryClient={qc}>
      {children}
    </KymaProvider>
  );
}

function frameStream(frames: Frame[]): Response {
  const enc = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const f of frames) {
        controller.enqueue(enc.encode(JSON.stringify(f) + "\n"));
      }
      controller.close();
    },
  });
  return new Response(body, {
    status: 200,
    headers: new Headers({ "content-type": "application/x-ndjson" }),
  });
}

const SAMPLE_FRAMES: Frame[] = [
  { type: "plan", sources: [{ source: "db.events", has_timestamp: true, timestamp_column: "ts" }] },
  { type: "source_progress", source: "db.events", state: "running" },
  { type: "rows", source: "db.events", rows: [{ ts: "2026-01-01", msg: "hello" }] },
  { type: "source_done", source: "db.events", total: 1, capped: false, dropped_clauses: [] },
  { type: "done", elapsed_ms: 42 },
];

describe("useKymaDiscover", () => {
  it("starts in idle state", () => {
    const qc = makeQC();
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise(() => {})));

    const { result } = renderHook(() => useKymaDiscover(), { wrapper: wrapper(qc) });

    expect(result.current.frames).toEqual([]);
    expect(result.current.isStreaming).toBe(false);
  });

  it("search() accumulates frames from the stream", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockReturnValue(Promise.resolve(frameStream(SAMPLE_FRAMES))),
    );

    const { result } = renderHook(() => useKymaDiscover(), { wrapper: wrapper(qc) });

    await act(async () => {
      await result.current.search({ query: "error", scope: { kind: "all" } });
    });

    expect(result.current.isStreaming).toBe(false);
    expect(result.current.frames).toHaveLength(SAMPLE_FRAMES.length);
    expect(result.current.frames[0].type).toBe("plan");
    expect(result.current.frames.at(-1)?.type).toBe("done");
  });

  it("abort() stops streaming", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((_url: string, init: RequestInit) => {
        // Slow stream that checks for abort
        const enc = new TextEncoder();
        const body = new ReadableStream<Uint8Array>({
          start(ctrl) {
            ctrl.enqueue(enc.encode(JSON.stringify(SAMPLE_FRAMES[0]) + "\n"));
            // Never enqueue more — simulates blocking
          },
        });
        init.signal?.addEventListener("abort", () => {
          // Nothing — the reader will just get a cancelled read
        });
        return Promise.resolve(
          new Response(body, {
            status: 200,
            headers: new Headers({ "content-type": "application/x-ndjson" }),
          }),
        );
      }),
    );

    const { result } = renderHook(() => useKymaDiscover(), { wrapper: wrapper(qc) });

    act(() => {
      void result.current.search({ query: "all", scope: { kind: "all" } });
    });

    await waitFor(() => expect(result.current.isStreaming).toBe(true));

    act(() => {
      result.current.abort();
    });

    await waitFor(() => expect(result.current.isStreaming).toBe(false), { timeout: 3000 });
  });

  it("search() resets frames on a new call", async () => {
    const qc = makeQC();
    let call = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => {
        call++;
        if (call === 1) {
          return Promise.resolve(frameStream([{ type: "done", elapsed_ms: 1 }]));
        }
        return Promise.resolve(frameStream([{ type: "done", elapsed_ms: 2 }, { type: "heartbeat" }]));
      }),
    );

    const { result } = renderHook(() => useKymaDiscover(), { wrapper: wrapper(qc) });

    await act(async () => {
      await result.current.search({ query: "q1", scope: { kind: "all" } });
    });
    expect(result.current.frames).toHaveLength(1);

    await act(async () => {
      await result.current.search({ query: "q2", scope: { kind: "all" } });
    });
    expect(result.current.frames).toHaveLength(2);
    expect(result.current.frames[0].type).toBe("done");
  });

  it("surfaces error when fetch returns non-OK", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({ error: { code: "bad_query", message: "syntax error" } }),
          {
            status: 400,
            headers: new Headers({ "content-type": "application/json" }),
          },
        ),
      ),
    );

    const { result } = renderHook(() => useKymaDiscover(), { wrapper: wrapper(qc) });

    await act(async () => {
      try {
        await result.current.search({ query: "!!", scope: { kind: "all" } });
      } catch { /* expected */ }
    });

    expect(result.current.error).toBeTruthy();
    expect(result.current.isStreaming).toBe(false);
  });
});
