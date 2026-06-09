/**
 * useKymaQuery tests.
 *
 * runQuery is an async generator over NDJSON. We fake it by returning a
 * Response with a ReadableStream body, which the transport's reader loop
 * consumes normally inside the jsdom environment (TextDecoder / ReadableStream
 * both available there).
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, cleanup, waitFor, act } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { KymaProvider } from "../provider/KymaProvider";
import { useKymaQuery } from "./useKymaQuery";

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

function makeNdjsonResponse(rows: Record<string, unknown>[]) {
  const ndjson = rows.map((r) => JSON.stringify(r)).join("\n");
  return Promise.resolve(
    new Response(ndjson, {
      status: 200,
      headers: new Headers({ "content-type": "application/x-ndjson" }),
    }),
  );
}

describe("useKymaQuery", () => {
  it("starts in idle state before execute is called", () => {
    const qc = makeQC();
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise(() => {})));

    const { result } = renderHook(() => useKymaQuery(), { wrapper: wrapper(qc) });

    expect(result.current.isRunning).toBe(false);
    expect(result.current.rows).toEqual([]);
    expect(result.current.columns).toEqual([]);
    expect(result.current.error).toBeNull();
  });

  it("execute runs a query and accumulates rows + columns", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() =>
        makeNdjsonResponse([
          { ts: "2026-01-01T00:00:00", level: "info", msg: "hello" },
          { ts: "2026-01-01T00:01:00", level: "warn", msg: "world" },
        ]),
      ),
    );

    const { result } = renderHook(() => useKymaQuery(), { wrapper: wrapper(qc) });

    await act(async () => {
      await result.current.execute({ database: "db1", query: "events | take 2", language: "kql" });
    });

    expect(result.current.isRunning).toBe(false);
    expect(result.current.rows).toHaveLength(2);
    expect(result.current.columns.map((c) => c.name)).toEqual(["ts", "level", "msg"]);
    expect(result.current.error).toBeNull();
  });

  it("surfaces error when query fails", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response("parse error near 'wat'", { status: 400 }),
      ),
    );

    const { result } = renderHook(() => useKymaQuery(), { wrapper: wrapper(qc) });

    await act(async () => {
      try {
        await result.current.execute({ database: "db1", query: "wat", language: "kql" });
      } catch { /* expected */ }
    });

    expect(result.current.error).toBeTruthy();
    expect(result.current.isRunning).toBe(false);
  });

  it("cancel() aborts an in-flight query", async () => {
    const qc = makeQC();
    // Slow fetch that never resolves until signal fires
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((_url: string, init: RequestInit) => {
        return new Promise<Response>((_resolve, reject) => {
          init.signal?.addEventListener("abort", () => {
            reject(new DOMException("aborted", "AbortError"));
          });
        });
      }),
    );

    const { result } = renderHook(() => useKymaQuery(), { wrapper: wrapper(qc) });

    act(() => {
      void result.current.execute({ database: "db1", query: "slow", language: "kql" });
    });

    await waitFor(() => expect(result.current.isRunning).toBe(true));

    act(() => {
      result.current.cancel();
    });

    await waitFor(() => expect(result.current.isRunning).toBe(false), { timeout: 3000 });
  });

  it("execute resets previous results before running", async () => {
    const qc = makeQC();
    let call = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => {
        call++;
        if (call === 1) {
          return makeNdjsonResponse([{ a: 1 }]);
        }
        return makeNdjsonResponse([{ b: 2 }, { b: 3 }]);
      }),
    );

    const { result } = renderHook(() => useKymaQuery(), { wrapper: wrapper(qc) });

    await act(async () => {
      await result.current.execute({ database: "db1", query: "q1", language: "kql" });
    });
    expect(result.current.rows).toHaveLength(1);

    await act(async () => {
      await result.current.execute({ database: "db1", query: "q2", language: "kql" });
    });
    expect(result.current.rows).toHaveLength(2);
  });
});
