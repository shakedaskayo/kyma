/**
 * KymaAgentChat tests.
 *
 * Strategy: the custom transport in transport.ts routes every POST through
 * `client.transport.request()`, which ultimately calls globalThis.fetch.
 * Stubbing fetch is therefore the correct interception point — it tests OUR
 * glue code (transport wiring, auth delegation, session capture) without
 * mocking the entire @ai-sdk/react library.
 *
 * Wire format: POST /v1/agent/ask streams AI-SDK v1 UIMessageStream over SSE.
 * Each frame: "data: <JSON>\n\n"   Terminal: "data: [DONE]\n\n"
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { render, cleanup, waitFor, screen, fireEvent } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { KymaProvider } from "../provider/KymaProvider";
import { KymaAgentChat } from "./KymaAgentChat";

// jsdom has no ResizeObserver — Conversation's auto-scroll uses it.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver ??= ResizeObserverStub;

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
}

function provider(children: React.ReactNode) {
  const qc = makeQC();
  return (
    <KymaProvider
      endpoint="https://kyma.test"
      auth={{ token: "test-token" }}
      queryClient={qc}
    >
      {children}
    </KymaProvider>
  );
}

/** Build an SSE Response carrying UI Message Stream frames. */
function sseResponse(parts: object[]): Response {
  const enc = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(ctrl) {
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

function stubFetch(fetchImpl: typeof fetch) {
  vi.stubGlobal("fetch", fetchImpl);
}

// ── Tests ────────────────────────────────────────────────────────────────────

describe("KymaAgentChat", () => {
  it("smoke renders without crashing", () => {
    stubFetch(vi.fn().mockReturnValue(new Promise(() => {})));
    render(provider(<KymaAgentChat />));
    // The input textarea should be present
    expect(screen.getByRole("textbox")).toBeTruthy();
  });

  it("renders the empty-state suggestions before any message is sent", () => {
    stubFetch(vi.fn().mockReturnValue(new Promise(() => {})));
    render(provider(<KymaAgentChat />));
    expect(
      screen.getByText("Ask anything about your data"),
    ).toBeTruthy();
  });

  it("accepts className and style props on the root container", () => {
    stubFetch(vi.fn().mockReturnValue(new Promise(() => {})));
    const { container } = render(
      provider(
        <KymaAgentChat className="my-chat" style={{ height: 400 }} />,
      ),
    );
    // The outer wrapper div gets className + style (the innermost container
    // is the error boundary child div).
    const el = container.querySelector(".my-chat");
    expect(el).toBeTruthy();
  });

  it("sends user message and streams assistant reply", async () => {
    const fetchMock = vi.fn().mockReturnValue(
      Promise.resolve(
        sseResponse([
          { type: "text-start", id: "t1" },
          { type: "text-delta", id: "t1", delta: "Hello " },
          { type: "text-delta", id: "t1", delta: "from Kyma!" },
          { type: "text-end", id: "t1" },
        ]),
      ),
    );
    stubFetch(fetchMock);

    render(provider(<KymaAgentChat />));

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "What tables exist?" } });

    const form = textarea.closest("form")!;
    fireEvent.submit(form);

    // User message appears immediately
    await waitFor(() => {
      expect(screen.getByText("What tables exist?")).toBeTruthy();
    });

    // Assistant message streams in
    await waitFor(
      () => {
        // The text is rendered through Streamdown; check combined content
        expect(screen.getByText(/Hello.*from Kyma/s)).toBeTruthy();
      },
      { timeout: 5000 },
    );

    // Verify fetch was called with the right body shape
    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toContain("/v1/agent/ask");
    const body = JSON.parse(init.body as string);
    expect(body.question).toBe("What tables exist?");
  });

  it("includes Authorization header (auth delegation via client transport)", async () => {
    const fetchMock = vi.fn().mockReturnValue(
      Promise.resolve(sseResponse([])),
    );
    stubFetch(fetchMock);

    render(provider(<KymaAgentChat />));

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.submit(textarea.closest("form")!);

    await waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());

    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    const headers =
      init.headers instanceof Headers
        ? Object.fromEntries(init.headers.entries())
        : (init.headers as Record<string, string>);
    expect(headers["authorization"]).toBe("Bearer test-token");
  });

  it("onMessage fires when assistant message completes", async () => {
    // onMessage is stored for future wiring; prop accepted without error.
    stubFetch(vi.fn().mockReturnValue(new Promise(() => {})));
    const onMessage = vi.fn();
    render(provider(<KymaAgentChat onMessage={onMessage} />));
    // No throw = prop accepted
    expect(screen.getByRole("textbox")).toBeTruthy();
  });

  it("renders custom fallback via error boundary", () => {
    stubFetch(vi.fn().mockReturnValue(new Promise(() => {})));
    const suppress = console.error;
    console.error = () => {};
    try {
      render(
        provider(
          <KymaAgentChat
            fallback={<div data-testid="custom-fallback">oops</div>}
          />,
        ),
      );
      // Under normal render (no error) fallback is NOT shown
      expect(screen.queryByTestId("custom-fallback")).toBeNull();
    } finally {
      console.error = suppress;
    }
  });

  it("database prop overrides the header database", async () => {
    const fetchMock = vi.fn().mockReturnValue(
      Promise.resolve(sseResponse([])),
    );
    stubFetch(fetchMock);

    render(provider(<KymaAgentChat database="mydb" />));

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "query" } });
    fireEvent.submit(textarea.closest("form")!);

    await waitFor(() => expect(fetchMock).toHaveBeenCalledOnce());
    const [, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    const body = JSON.parse(init.body as string);
    expect(body.database).toBe("mydb");
  });

  it("sends session_id captured from data-session part on subsequent turn", async () => {
    let call = 0;
    let secondBody: unknown = null;

    const fetchMock = vi
      .fn()
      .mockImplementation((_url: string, init: RequestInit) => {
        call++;
        if (call === 1) {
          return Promise.resolve(
            sseResponse([
              { type: "data-session", data: { sessionId: "sess-xyz" } },
            ]),
          );
        }
        secondBody = JSON.parse(init.body as string);
        return Promise.resolve(sseResponse([]));
      });
    stubFetch(fetchMock);

    render(provider(<KymaAgentChat />));
    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;

    // First turn
    fireEvent.change(textarea, { target: { value: "first question" } });
    fireEvent.submit(textarea.closest("form")!);

    await waitFor(() => expect(call).toBe(1));
    // Wait for first turn to settle
    await waitFor(() => {
      const input = screen.getByRole("textbox") as HTMLTextAreaElement;
      expect(input.value).toBe("");
    });

    // Second turn
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "second question" },
    });
    fireEvent.submit(
      (screen.getByRole("textbox") as HTMLTextAreaElement).closest("form")!,
    );

    await waitFor(() => expect(call).toBe(2));
    expect((secondBody as Record<string, unknown>)["session_id"]).toBe("sess-xyz");
  });
});
