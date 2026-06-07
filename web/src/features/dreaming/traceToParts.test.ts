import { expect, test } from "vitest";
import { traceToParts } from "./traceToParts";
import type { TraceFrame } from "@/sdk/dreaming";

const f = (event: string, data: Record<string, unknown> = {}): TraceFrame => ({ event, data });

test("coalesces consecutive answer deltas into one text block", () => {
  const parts = traceToParts([
    f("session", { session_id: "s1" }),
    f("run_started"),
    f("answer_delta", { text: "Hello " }),
    f("answer_delta", { text: "world" }),
  ]);
  expect(parts).toEqual([{ kind: "text", text: "Hello world" }]);
});

test("coalesces thinking deltas into reasoning, separate from text", () => {
  const parts = traceToParts([
    f("thinking_delta", { text: "let me " }),
    f("thinking_delta", { text: "think" }),
    f("answer_delta", { text: "done" }),
  ]);
  expect(parts).toEqual([
    { kind: "reasoning", text: "let me think" },
    { kind: "text", text: "done" },
  ]);
});

test("pairs tool_call with its FIFO tool_result", () => {
  const parts = traceToParts([
    f("tool_call", { tool: "run_sql", args: { q: "select 1" }, call_index: 0 }),
    f("tool_result", { tool: "run_sql", result: { rows: 1 } }),
  ]);
  expect(parts).toHaveLength(1);
  expect(parts[0]).toMatchObject({
    kind: "tool",
    tool: "run_sql",
    input: { q: "select 1" },
    output: { rows: 1 },
    state: "output-available",
  });
});

test("matches multiple same-named tools FIFO", () => {
  const parts = traceToParts([
    f("tool_call", { tool: "t", args: { i: 1 } }),
    f("tool_call", { tool: "t", args: { i: 2 } }),
    f("tool_result", { tool: "t", result: "first" }),
    f("tool_result", { tool: "t", result: "second" }),
  ]);
  expect(parts).toHaveLength(2);
  expect(parts[0]).toMatchObject({ input: { i: 1 }, output: "first" });
  expect(parts[1]).toMatchObject({ input: { i: 2 }, output: "second" });
});

test("leaves an unmatched tool_call open (input-available)", () => {
  const parts = traceToParts([f("tool_call", { tool: "t", args: {} })]);
  expect(parts[0]).toMatchObject({ kind: "tool", state: "input-available" });
});

test("run_error becomes an error part", () => {
  const parts = traceToParts([
    f("answer_delta", { text: "partial" }),
    f("run_error", { code: "timeout", message: "took too long" }),
  ]);
  expect(parts[1]).toEqual({ kind: "error", code: "timeout", message: "took too long" });
});

test("interleaved text → tool → text preserves order", () => {
  const parts = traceToParts([
    f("answer_delta", { text: "before " }),
    f("tool_call", { tool: "t", args: {} }),
    f("tool_result", { tool: "t", result: "ok" }),
    f("answer_delta", { text: "after" }),
  ]);
  expect(parts.map((p) => p.kind)).toEqual(["text", "tool", "text"]);
  expect((parts[2] as { text: string }).text).toBe("after");
});

test("empty trace yields no parts", () => {
  expect(traceToParts([])).toEqual([]);
});
