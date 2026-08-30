/**
 * usePensieveAgent tests.
 *
 * The server sends AI-SDK v1 UIMessageStream over SSE:
 *   Content-Type: text/event-stream
 *   x-vercel-ai-ui-message-stream: v1
 *
 * Each frame is: "data: <JSON>\n\n"
 * Terminal frame: "data: [DONE]\n\n"
 *
 * We stub fetch to return a Response with a ReadableStream body emitting
 * these SSE events.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, cleanup, waitFor, act } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { PensieveProvider } from "../provider/PensieveProvider";
import { usePensieveAgent } from "./usePensieveAgent";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
}

function wrapper(qc: QueryClient) {
  return ({ children }: { children: React.ReactNode }) => (
    <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
      {children}
    </PensieveProvider>
  );
}

/** Build an SSE Response carrying UI Message Stream frames. */
function sseResponse(parts: object[]): Response {
  const enc = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(ctrl) {
      // start / finish wrapper
      const frames = [
        { type: "start", messageId: "msg-1" },
        ...parts,
        { type: "finish" },
      ];
      for (const f of frames) {
        ctrl.enqueue(enc.encode(`data: ${JSON.stringify(f)}\n\n`));
      }
      ctrl.enqueue(enc.encode("data: [DONE]\n\n"));
      ctrl.close();
    },
  });
  return new Response(body, {
    status: 200,
    headers: new Headers({
      "content-type": "text/event-stream",
      "x-vercel-ai-ui-message-stream": "v1",
    }),
  });
}

describe("usePensieveAgent", () => {
  it("starts with empty messages and idle status", () => {
    const qc = makeQC();
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise(() => {})));

    const { result } = renderHook(() => usePensieveAgent(), { wrapper: wrapper(qc) });

    expect(result.current.messages).toEqual([]);
    expect(result.current.status).toBe("idle");
  });

  it("send() adds a user message and streams assistant reply via text-delta", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockReturnValue(
        Promise.resolve(
          sseResponse([
            { type: "text-start", id: "t1" },
            { type: "text-delta", id: "t1", delta: "Hello " },
            { type: "text-delta", id: "t1", delta: "world" },
            { type: "text-end", id: "t1" },
          ]),
        ),
      ),
    );

    const { result } = renderHook(() => usePensieveAgent(), { wrapper: wrapper(qc) });

    await act(async () => {
      await result.current.send("Hi there");
    });

    expect(result.current.messages).toHaveLength(2);
    const user = result.current.messages[0];
    expect(user.role).toBe("user");
    expect(user.text).toBe("Hi there");

    const assistant = result.current.messages[1];
    expect(assistant.role).toBe("assistant");
    expect(assistant.text).toBe("Hello world");
    expect(result.current.status).toBe("idle");
  });

  it("status transitions to 'streaming' while a request is in-flight", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => {
        // Return a stream that doesn't close immediately
        const enc = new TextEncoder();
        const body = new ReadableStream<Uint8Array>({
          start(ctrl) {
            ctrl.enqueue(enc.encode(`data: ${JSON.stringify({ type: "start", messageId: "m" })}\n\n`));
            // Never close — simulates slow stream
          },
        });
        return Promise.resolve(
          new Response(body, {
            status: 200,
            headers: new Headers({
              "content-type": "text/event-stream",
              "x-vercel-ai-ui-message-stream": "v1",
            }),
          }),
        );
      }),
    );

    const { result } = renderHook(() => usePensieveAgent(), { wrapper: wrapper(qc) });

    act(() => {
      void result.current.send("question");
    });

    await waitFor(() => expect(result.current.status).toBe("streaming"));
  });

  it("stop() aborts in-flight stream", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((_url: string, init: RequestInit) => {
        const enc = new TextEncoder();
        const body = new ReadableStream<Uint8Array>({
          start(ctrl) {
            ctrl.enqueue(enc.encode(`data: ${JSON.stringify({ type: "start", messageId: "m" })}\n\n`));
            init.signal?.addEventListener("abort", () => {
              ctrl.error(new DOMException("aborted", "AbortError"));
            });
          },
        });
        return Promise.resolve(
          new Response(body, {
            status: 200,
            headers: new Headers({
              "content-type": "text/event-stream",
              "x-vercel-ai-ui-message-stream": "v1",
            }),
          }),
        );
      }),
    );

    const { result } = renderHook(() => usePensieveAgent(), { wrapper: wrapper(qc) });

    act(() => {
      void result.current.send("question");
    });

    await waitFor(() => expect(result.current.status).toBe("streaming"));

    act(() => {
      result.current.stop();
    });

    await waitFor(() => expect(result.current.status).toBe("idle"), { timeout: 3000 });
  });

  it("captures session_id from data-session part and echoes on next send", async () => {
    const qc = makeQC();
    let call = 0;
    let capturedBody: string | null = null;

    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((_url: string, init: RequestInit) => {
        call++;
        if (call === 1) {
          return Promise.resolve(
            sseResponse([
              { type: "data-session", data: { sessionId: "sess-abc" } },
            ]),
          );
        }
        capturedBody = init.body as string;
        return Promise.resolve(sseResponse([]));
      }),
    );

    const { result } = renderHook(() => usePensieveAgent(), { wrapper: wrapper(qc) });

    await act(async () => {
      await result.current.send("first");
    });
    await act(async () => {
      await result.current.send("second");
    });

    expect(capturedBody).not.toBeNull();
    const parsed = JSON.parse(capturedBody!);
    expect(parsed.session_id).toBe("sess-abc");
  });

  it("rapid double send(): aborted first run cannot flip status while second streams", async () => {
    const qc = makeQC();
    // First call: hangs until aborted. Second call: a never-ending stream so
    // status must still be "streaming" when the first run's AbortError lands.
    let call = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((_url: string, init?: RequestInit) => {
        call += 1;
        if (call === 1) {
          return new Promise<Response>((_resolve, reject) => {
            init?.signal?.addEventListener("abort", () =>
              reject(new DOMException("aborted", "AbortError")),
            );
          });
        }
        // Open stream that never closes — keeps run 2 in "streaming".
        const body = new ReadableStream<Uint8Array>({ start() {} });
        return Promise.resolve(
          new Response(body, {
            status: 200,
            headers: new Headers({ "content-type": "text/event-stream" }),
          }),
        );
      }),
    );
    const { result } = renderHook(() => usePensieveAgent(), { wrapper: wrapper(qc) });

    act(() => {
      void result.current.send("first");
    });
    act(() => {
      void result.current.send("second"); // aborts run 1
    });

    await waitFor(() => expect(result.current.status).toBe("streaming"));
    // Give run 1's AbortError catch a chance to (incorrectly) flip status.
    await act(() => new Promise((r) => setTimeout(r, 20)));
    expect(result.current.status).toBe("streaming");
  });

  it("error status when fetch returns non-OK", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response("Internal Server Error", { status: 500 })),
    );

    const { result } = renderHook(() => usePensieveAgent(), { wrapper: wrapper(qc) });

    await act(async () => {
      try {
        await result.current.send("hello");
      } catch { /* expected */ }
    });

    expect(result.current.status).toBe("error");
  });
});
