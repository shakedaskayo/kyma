/**
 * usePensieveAgent — headless hook for the Pensieve Ask agent.
 *
 * Wire format: POST /v1/agent/ask returns SSE (text/event-stream) carrying
 * the Vercel AI SDK "UI Message Stream v1" protocol:
 *   x-vercel-ai-ui-message-stream: v1
 *   Content-Type: text/event-stream
 *
 * Each SSE event is "data: <JSON>\n\n". Terminal event: "data: [DONE]\n\n".
 *
 * Part types parsed (matching server's ui_stream.rs):
 *   { type: "start", messageId }
 *   { type: "text-start", id }
 *   { type: "text-delta", id, delta }
 *   { type: "text-end", id }
 *   { type: "data-<name>", data }    — data-session (session resume), data-usage
 *   { type: "error", errorText }
 *   { type: "finish" | "finish-step" | "start-step" }
 *   [DONE] — terminal sentinel
 *
 * Auth is handled by routing the POST through client.transport.request(),
 * which injects Authorization and x-database headers automatically.
 *
 * This hook does NOT pull in @ai-sdk/react to keep @pensieve-ai/react lean.
 * The /agent subpath export is reserved for a future richer integration.
 *
 * Request body (AskRequest — server's expected JSON):
 *   { question, database?, include_thinking?, session_id? }
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { usePensieveClient } from "../provider/context";

// ── Public types ──────────────────────────────────────────────────────────────

/** A single chat message in the conversation. */
export interface AgentMessage {
  id: string;
  role: "user" | "assistant";
  /** Completed text content of the message. */
  text: string;
  /** Partially accumulated text while streaming (not yet in `text`). */
  streamingText?: string;
}

export type AgentStatus = "idle" | "streaming" | "error";

export interface UsePensieveAgentArgs {
  database?: string;
  includeThinking?: boolean;
}

export interface UsePensieveAgentResult {
  messages: AgentMessage[];
  status: AgentStatus;
  /**
   * Send a user message and stream the assistant reply.
   * Returns a promise that resolves when the stream is complete.
   */
  send: (text: string) => Promise<void>;
  /** Abort the current in-flight stream. No-op if not streaming. */
  stop: () => void;
}

// ── SSE parser ────────────────────────────────────────────────────────────────

type UiPart = Record<string, unknown>;

/** Parse SSE data frames from a ReadableStream. Yields each parsed JSON part,
 *  or `null` for the [DONE] sentinel. Stops on abort. */
async function* parseSseStream(
  body: ReadableStream<Uint8Array>,
  signal: AbortSignal,
): AsyncGenerator<UiPart | null, void, void> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buf = "";

  try {
    while (true) {
      if (signal.aborted) break;

      let readResult: ReadableStreamReadResult<Uint8Array>;
      try {
        readResult = await reader.read();
      } catch (err) {
        if ((err as DOMException)?.name === "AbortError") break;
        throw err;
      }

      const { value, done } = readResult;
      if (value) buf += decoder.decode(value, { stream: true });

      // Parse SSE events separated by "\n\n"
      let sep: number;
      while ((sep = buf.indexOf("\n\n")) >= 0) {
        const block = buf.slice(0, sep);
        buf = buf.slice(sep + 2);
        for (const line of block.split("\n")) {
          if (!line.startsWith("data:")) continue;
          const payload = line.slice(5).trim();
          if (payload === "[DONE]") {
            yield null; // sentinel
            return;
          }
          try {
            yield JSON.parse(payload) as UiPart;
          } catch { /* skip malformed frames */ }
        }
      }

      if (done) break;
    }
  } finally {
    reader.releaseLock();
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function newId(): string {
  return Math.random().toString(36).slice(2, 10);
}

// ── Implementation ────────────────────────────────────────────────────────────

export function usePensieveAgent(args?: UsePensieveAgentArgs): UsePensieveAgentResult {
  const client = usePensieveClient();

  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [status, setStatus] = useState<AgentStatus>("idle");

  const abortRef = useRef<AbortController | null>(null);
  /** Server-assigned conversation id for multi-turn resume. */
  const sessionIdRef = useRef<string | null>(null);

  // Abort on unmount.
  useEffect(() => {
    return () => {
      abortRef.current?.abort();
    };
  }, []);

  const stop = useCallback(() => {
    abortRef.current?.abort();
    abortRef.current = null;
    setStatus("idle");
  }, []);

  const send = useCallback(
    async (text: string): Promise<void> => {
      // Abort any in-flight stream.
      abortRef.current?.abort();
      const ac = new AbortController();
      abortRef.current = ac;

      // Append user message immediately.
      const userMsgId = newId();
      setMessages((prev) => [
        ...prev,
        { id: userMsgId, role: "user", text },
      ]);

      // Prepare assistant message slot.
      const assistantMsgId = newId();
      setMessages((prev) => [
        ...prev,
        { id: assistantMsgId, role: "assistant", text: "", streamingText: "" },
      ]);

      setStatus("streaming");

      // Route through the transport so auth headers are injected automatically.
      const requestBody = JSON.stringify({
        question: text,
        database: args?.database ?? client.transport.database ?? undefined,
        include_thinking: args?.includeThinking ?? false,
        session_id: sessionIdRef.current ?? undefined,
      });

      let res: Response;
      try {
        res = await client.transport.request("/v1/agent/ask", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: requestBody,
          signal: ac.signal,
        });
      } catch (err) {
        if ((err as DOMException)?.name === "AbortError") {
          // Only the still-current run may flip status — a rapid second
          // send() aborts this one while its own stream is in flight.
          if (abortRef.current === ac) setStatus("idle");
          return;
        }
        if (abortRef.current === ac) setStatus("error");
        throw err;
      }

      if (!res.ok) {
        setStatus("error");
        const detail = await res.text().catch(() => "");
        throw new Error(`agent ask failed (${res.status}): ${detail.slice(0, 200)}`);
      }

      if (!res.body) {
        setStatus("error");
        throw new Error("agent ask: empty response body");
      }

      // Track per-text-part deltas by part id.
      const textParts = new Map<string, string>();

      try {
        for await (const part of parseSseStream(res.body, ac.signal)) {
          if (part === null) break; // [DONE]

          const type = part.type as string;

          if (type === "text-start") {
            textParts.set(part.id as string, "");
          } else if (type === "text-delta") {
            const id = part.id as string;
            const delta = (part.delta as string) ?? "";
            textParts.set(id, (textParts.get(id) ?? "") + delta);

            const combined = [...textParts.values()].join("");
            setMessages((msgs) =>
              msgs.map((m) =>
                m.id === assistantMsgId ? { ...m, streamingText: combined } : m,
              ),
            );
          } else if (type === "text-end" || type === "finish") {
            const combined = [...textParts.values()].join("");
            setMessages((msgs) =>
              msgs.map((m) =>
                m.id === assistantMsgId
                  ? { ...m, text: combined, streamingText: undefined }
                  : m,
              ),
            );
          } else if (type === "data-session") {
            // Capture session id for multi-turn resume.
            const data = part.data as { sessionId?: string } | undefined;
            if (typeof data?.sessionId === "string" && data.sessionId.length > 0) {
              sessionIdRef.current = data.sessionId;
            }
          }
          // Other parts (reasoning-*, tool-*, data-usage, start, start-step,
          // finish-step) are silently consumed — Phase 5 components can
          // access richer data by subscribing to the raw messages array.
        }

        if (abortRef.current === ac) {
          setStatus("idle");
          abortRef.current = null;
        }
      } catch (err) {
        if ((err as DOMException)?.name === "AbortError") {
          // Only the still-current run may flip status — a rapid second
          // send() aborts this one while its own stream is in flight.
          if (abortRef.current === ac) setStatus("idle");
          return;
        }
        if (abortRef.current === ac) setStatus("error");
        throw err;
      }
    },
    [client, args],
  );

  return { messages, status, send, stop };
}
