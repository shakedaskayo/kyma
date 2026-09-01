import { DefaultChatTransport, type UIMessage } from "ai";

/** Per-turn request options posted to `/v1/agent/ask`. */
export type AgentRequestOptions = {
  database?: string;
  include_thinking?: boolean;
  /** Conversation session id, echoed back from a prior `data-session` part. */
  session_id?: string;
};

/**
 * Build a `useChat` transport for the Pensieve agent endpoint. We don't post the
 * full AI-SDK `messages` array — the backend only needs the latest question
 * plus a few options (and resumes prior context server-side via `session_id`),
 * so `prepareSendMessagesRequest` sends exactly the existing `AskRequest` shape.
 *
 * `getOptions` is read at send time so callers can vary the database, thinking
 * toggle, or session id between turns without rebuilding the transport.
 */
export function createAgentTransport(
  endpoint: string | undefined,
  token: string | undefined,
  getOptions: () => AgentRequestOptions,
) {
  const base = (endpoint ?? "").replace(/\/$/, "");
  return new DefaultChatTransport<UIMessage>({
    api: `${base}/v1/agent/ask`,
    headers: token ? { Authorization: `Bearer ${token}` } : undefined,
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
