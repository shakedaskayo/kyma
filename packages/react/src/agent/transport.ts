import { DefaultChatTransport, type UIMessage } from "ai";
import type { PensieveClient } from "@pensieve-ai/client";

/** Per-turn request options posted to `/v1/agent/ask`. */
export type AgentRequestOptions = {
  database?: string;
  include_thinking?: boolean;
  /** Conversation session id, echoed back from a prior `data-session` part. */
  session_id?: string;
};

/**
 * Build a `useChat` transport for the Pensieve agent endpoint.
 *
 * Instead of carrying endpoint/token directly, we route every fetch through
 * `client.transport.request()` so auth/refresh (including JWT expiry and token
 * rotation) is handled by the same transport that all other Pensieve hooks use.
 *
 * `prepareSendMessagesRequest` sends exactly the existing AskRequest shape:
 * only the latest question + a few options. The backend does not need the full
 * messages array — it resumes server-side via `session_id`.
 *
 * `getOptions` is read at send time so callers can vary the database, thinking
 * toggle, or session id between turns without rebuilding the transport.
 */
export function createAgentTransport(
  client: PensieveClient,
  getOptions: () => AgentRequestOptions,
) {
  const base = client.transport.endpoint.replace(/\/$/, "");
  return new DefaultChatTransport<UIMessage>({
    api: `${base}/v1/agent/ask`,
    // Custom fetch: route through the client transport so auth headers
    // (Authorization + x-database) are injected and tokens are refreshed.
    fetch: async (input, init) => {
      // input is the full URL string; strip the base to get the path.
      const url = typeof input === "string" ? input : (input as URL).toString();
      const path = url.startsWith(base)
        ? url.slice(base.length)
        : url;
      return client.transport.request(path, {
        ...(init ?? {}),
        headers: init?.headers as HeadersInit | undefined,
      });
    },
    prepareSendMessagesRequest: ({ messages }) => {
      const last = messages[messages.length - 1];
      const question = (last?.parts ?? [])
        .filter((p) => p.type === "text")
        .map((p) => (p as { text: string }).text)
        .join("");
      const opts = getOptions();
      return {
        body: {
          question,
          database: opts.database || undefined,
          include_thinking: opts.include_thinking ?? false,
          session_id: opts.session_id ?? undefined,
        },
      };
    },
  });
}
