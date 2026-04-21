import type { AgentEvent, AskAgentRequest } from "./types";

/**
 * Stream agent events from `POST /v1/agent/ask`.
 *
 * The backend returns `text/event-stream` with frames shaped as
 *   event: <name>\n
 *   data: <json>\n
 *   \n
 * We can't use the browser's `EventSource` because it only supports GET.
 * Instead we parse the SSE framing off `response.body.getReader()` ourselves.
 *
 * Each yielded value has `type` set to the SSE event name (so consumers can
 * switch on event.type just like a Rust enum), merged with the parsed JSON
 * payload.
 */
export async function* runAgent(
  endpoint: string,
  token: string | undefined,
  body: AskAgentRequest,
  signal: AbortSignal,
): AsyncGenerator<AgentEvent> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    Accept: "text/event-stream",
  };
  if (token) headers["Authorization"] = `Bearer ${token}`;

  const resp = await fetch(`${endpoint.replace(/\/$/, "")}/v1/agent/ask`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
    signal,
  });
  if (!resp.ok) {
    const detail = await resp.text().catch(() => "");
    throw new Error(`agent ask failed: ${resp.status} ${detail}`);
  }
  if (!resp.body) {
    throw new Error("agent ask returned no body");
  }

  const reader = resp.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    // SSE frames are separated by a blank line. Support both \n\n and \r\n\r\n.
    for (;;) {
      const boundary = nextFrameBoundary(buf);
      if (!boundary) break;
      const frame = buf.slice(0, boundary.start);
      buf = buf.slice(boundary.end);
      const parsed = parseFrame(frame);
      if (parsed) yield parsed;
    }
  }
  // Flush any trailing frame not terminated by a blank line.
  const tail = buf.trim();
  if (tail) {
    const parsed = parseFrame(tail);
    if (parsed) yield parsed;
  }
}

function nextFrameBoundary(buf: string): { start: number; end: number } | null {
  const a = buf.indexOf("\n\n");
  const b = buf.indexOf("\r\n\r\n");
  if (a < 0 && b < 0) return null;
  if (b < 0 || (a >= 0 && a < b)) return { start: a, end: a + 2 };
  return { start: b, end: b + 4 };
}

function parseFrame(frame: string): AgentEvent | null {
  let eventName = "message";
  let data = "";
  for (const rawLine of frame.split(/\r?\n/)) {
    const line = rawLine;
    if (line.startsWith("event:")) eventName = line.slice(6).trim();
    else if (line.startsWith("data:")) data += (data ? "\n" : "") + line.slice(5).trim();
    // ignore comments (":...") and other fields (id:, retry:)
  }
  if (!data) return null;
  try {
    const parsed = JSON.parse(data) as Record<string, unknown>;
    return { type: eventName, ...parsed } as AgentEvent;
  } catch {
    return null;
  }
}
