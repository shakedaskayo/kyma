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
  _sseCounter = 0;
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

let _sseCounter = 0;

/** Build an SSE Response carrying UI Message Stream frames. */
function sseResponse(parts: object[]): Response {
  const messageId = `msg-${++_sseCounter}`;
  const enc = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(ctrl) {
      const frames = [
        { type: "start", messageId },
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

/** URL-aware stub: serves capabilities JSON to the gate, fresh SSE per ask. */
function agentFetch(parts: object[]): ReturnType<typeof vi.fn> {
  return vi.fn().mockImplementation((url: string) => {
    if (String(url).includes("/v1/capabilities")) {
      return Promise.resolve(
        new Response(JSON.stringify({ mode: "server", agent: true }), {
          status: 200,
          headers: new Headers({ "content-type": "application/json" }),
        }),
      );
    }
    return Promise.resolve(sseResponse(parts));
  });
}

/** The /v1/agent/ask calls recorded by an agentFetch stub. */
function askCalls(fetchMock: ReturnType<typeof vi.fn>): [string, RequestInit][] {
  return (fetchMock.mock.calls as [string, RequestInit][]).filter(([u]) =>
    String(u).includes("/v1/agent/ask"),
  );
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
    const fetchMock = agentFetch([
      { type: "text-start", id: "t1" },
      { type: "text-delta", id: "t1", delta: "Hello " },
      { type: "text-delta", id: "t1", delta: "from Kyma!" },
      { type: "text-end", id: "t1" },
    ]);
    stubFetch(fetchMock as unknown as typeof fetch);

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

    // Verify the ask call carried the right body shape
    const asks = askCalls(fetchMock);
    expect(asks).toHaveLength(1);
    const body = JSON.parse(asks[0][1].body as string);
    expect(body.question).toBe("What tables exist?");
  });

  it("includes Authorization header (auth delegation via client transport)", async () => {
    const fetchMock = agentFetch([]);
    stubFetch(fetchMock as unknown as typeof fetch);

    render(provider(<KymaAgentChat />));

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "hello" } });
    fireEvent.submit(textarea.closest("form")!);

    await waitFor(() => expect(askCalls(fetchMock)).toHaveLength(1));

    const [, init] = askCalls(fetchMock)[0];
    const headers =
      init.headers instanceof Headers
        ? Object.fromEntries(init.headers.entries())
        : (init.headers as Record<string, string>);
    expect(headers["authorization"]).toBe("Bearer test-token");
  });

  it("onMessage fires once with assembled text when assistant turn completes", async () => {
    const fetchMock = agentFetch([
      { type: "text-start", id: "t1" },
      { type: "text-delta", id: "t1", delta: "Hello " },
      { type: "text-delta", id: "t1", delta: "world!" },
      { type: "text-end", id: "t1" },
    ]);
    stubFetch(fetchMock as unknown as typeof fetch);

    const onMessage = vi.fn();
    render(provider(<KymaAgentChat onMessage={onMessage} />));

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "ping" } });
    fireEvent.submit(textarea.closest("form")!);

    // Wait for the assistant reply to render
    await waitFor(
      () => {
        expect(screen.getByText(/Hello.*world/s)).toBeTruthy();
      },
      { timeout: 5000 },
    );

    // onMessage should have been called exactly once with the complete text
    await waitFor(
      () => {
        expect(onMessage).toHaveBeenCalledOnce();
      },
      { timeout: 5000 },
    );
    const [msg] = onMessage.mock.calls[0] as [{ role: string; text: string }];
    expect(msg.role).toBe("assistant");
    expect(msg.text).toContain("Hello");
    expect(msg.text).toContain("world!");
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
    const fetchMock = agentFetch([]);
    stubFetch(fetchMock as unknown as typeof fetch);

    render(provider(<KymaAgentChat database="mydb" />));

    const textarea = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: "query" } });
    fireEvent.submit(textarea.closest("form")!);

    await waitFor(() => expect(askCalls(fetchMock)).toHaveLength(1));
    const [, init] = askCalls(fetchMock)[0];
    const body = JSON.parse(init.body as string);
    expect(body.database).toBe("mydb");
  });

  it("sends session_id captured from data-session part on subsequent turn", async () => {
    let call = 0;
    let secondBody: unknown = null;

    const fetchMock = vi
      .fn()
      .mockImplementation((url: string, init: RequestInit) => {
        if (String(url).includes("/v1/capabilities")) {
          return Promise.resolve(
            new Response(JSON.stringify({ mode: "server", agent: true }), {
              status: 200,
              headers: new Headers({ "content-type": "application/json" }),
            }),
          );
        }
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
