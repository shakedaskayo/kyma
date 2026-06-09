/** Pre-built SSE UIMessageStream frames for agent fixture responses. */

export interface SseFrame {
  type: string;
  [key: string]: unknown;
}

let _counter = 0;

/** Build a complete SSE Response body string from a list of content frames. */
export function buildSseBody(contentFrames: SseFrame[]): string {
  const messageId = `fixture-msg-${++_counter}`;
  const frames: SseFrame[] = [
    { type: "start", messageId },
    ...contentFrames,
    { type: "finish" },
  ];
  return (
    frames.map((f) => `data: ${JSON.stringify(f)}\n\n`).join("") +
    "data: [DONE]\n\n"
  );
}

/** Build a Response that streams a UIMessageStream over SSE. */
export function buildSseResponse(contentFrames: SseFrame[]): Response {
  const enc = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(ctrl) {
      const messageId = `fixture-msg-${++_counter}`;
      const frames: SseFrame[] = [
        { type: "start", messageId },
        ...contentFrames,
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

/** Canned response for "What tables exist?" */
export const CANNED_TABLES_RESPONSE: SseFrame[] = [
  { type: "data-session", data: { sessionId: "story-session-1" } },
  { type: "text-start", id: "t1" },
  { type: "text-delta", id: "t1", delta: "In the **production** database, the following tables are available:\n\n" },
  { type: "text-delta", id: "t1", delta: "| Table | Row Count | Description |\n" },
  { type: "text-delta", id: "t1", delta: "|-------|-----------|-------------|\n" },
  { type: "text-delta", id: "t1", delta: "| `events` | ~2.4M | Application log events with severity, service, and message |\n" },
  { type: "text-delta", id: "t1", delta: "| `metrics` | ~890K | Time-series metric samples with labels |\n" },
  { type: "text-delta", id: "t1", delta: "| `traces` | ~320K | Distributed trace spans with duration and status |\n\n" },
  { type: "text-delta", id: "t1", delta: "You can query any of these using KQL. For example:\n\n```kql\nevents | take 10\n```" },
  { type: "text-end", id: "t1" },
];

/** Canned response for "Show me recent errors" */
export const CANNED_ERRORS_RESPONSE: SseFrame[] = [
  { type: "text-start", id: "t2" },
  { type: "text-delta", id: "t2", delta: "Here are the most recent errors from the **billing-service**:\n\n" },
  { type: "text-delta", id: "t2", delta: "1. `Payment gateway timeout` — 5.0s at 10:00:03 UTC\n" },
  { type: "text-delta", id: "t2", delta: "2. `Payment retry #2 for order 9821` — 245ms at 10:00:02 UTC\n\n" },
  { type: "text-delta", id: "t2", delta: "The gateway timeout suggests the upstream payment provider may be experiencing issues. " },
  { type: "text-delta", id: "t2", delta: "I'd recommend checking `/v1/metrics` for the `billing_gateway_latency_p99` metric." },
  { type: "text-end", id: "t2" },
];
