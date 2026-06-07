import type { TraceFrame } from "@/sdk/dreaming";

/**
 * Ordered parts folded from a recorded agent trace, ready for static render.
 * Coalesces consecutive answer/thinking deltas into single blocks; pairs
 * tool_call → tool_result FIFO per tool name; surfaces run_error as a banner.
 */
export type ConversationPart =
  | { kind: "text"; text: string }
  | { kind: "reasoning"; text: string }
  | {
      kind: "tool";
      tool: string;
      input: unknown;
      output?: unknown;
      state: "input-available" | "output-available" | "output-error";
      errorText?: string;
    }
  | { kind: "error"; code?: string; message: string };

const str = (v: unknown): string => (typeof v === "string" ? v : v == null ? "" : String(v));

/**
 * Fold a trace frame list into ordered conversation parts.
 *
 * - answer_delta / answer_final → text blocks (consecutive deltas coalesced).
 * - thinking_delta → reasoning blocks (consecutive deltas coalesced).
 * - tool_call → opens a tool part (state: input-available), tracked FIFO by name.
 * - tool_result → closes the oldest open tool part of that name
 *   (state: output-available). If none open, appends a closed tool part.
 * - run_error → an error banner part.
 *
 * `session` / `run_started` frames carry no rendered content and are skipped.
 */
export function traceToParts(trace: TraceFrame[]): ConversationPart[] {
  const parts: ConversationPart[] = [];
  // FIFO queue of indices into `parts` for each not-yet-closed tool, by name.
  const openTools = new Map<string, number[]>();

  for (const frame of trace ?? []) {
    const data = frame.data ?? {};
    switch (frame.event) {
      case "answer_delta":
      case "answer_final": {
        const text = str((data as { text?: unknown }).text);
        if (!text) break;
        const last = parts[parts.length - 1];
        if (last && last.kind === "text") {
          last.text += text;
        } else {
          parts.push({ kind: "text", text });
        }
        break;
      }
      case "thinking_delta": {
        const text = str((data as { text?: unknown }).text);
        if (!text) break;
        const last = parts[parts.length - 1];
        if (last && last.kind === "reasoning") {
          last.text += text;
        } else {
          parts.push({ kind: "reasoning", text });
        }
        break;
      }
      case "tool_call": {
        const tool = str((data as { tool?: unknown }).tool) || "tool";
        const input = (data as { args?: unknown }).args;
        const idx = parts.push({ kind: "tool", tool, input, state: "input-available" }) - 1;
        const q = openTools.get(tool) ?? [];
        q.push(idx);
        openTools.set(tool, q);
        break;
      }
      case "tool_result": {
        const tool = str((data as { tool?: unknown }).tool) || "tool";
        const result = (data as { result?: unknown }).result;
        const q = openTools.get(tool);
        if (q && q.length > 0) {
          const idx = q.shift()!;
          const part = parts[idx];
          if (part && part.kind === "tool") {
            part.output = result;
            part.state = "output-available";
          }
        } else {
          // No matching open call — render a standalone completed tool part.
          parts.push({ kind: "tool", tool, input: undefined, output: result, state: "output-available" });
        }
        break;
      }
      case "run_error": {
        const message = str((data as { message?: unknown }).message) || "Run error";
        const code = (data as { code?: unknown }).code;
        parts.push({ kind: "error", message, code: code == null ? undefined : str(code) });
        break;
      }
      // session / run_started / unknown — no rendered content.
      default:
        break;
    }
  }

  return parts;
}
