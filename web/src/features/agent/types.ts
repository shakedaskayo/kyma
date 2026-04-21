export type RunStarted = { type: "run_started"; run_id: string; model: string; question: string };
export type ToolCall = { type: "tool_call"; tool: string; args: unknown; call_index: number };
export type ToolResult = { type: "tool_result"; tool: string; result: unknown };
export type ThinkingDelta = { type: "thinking_delta"; text: string };
export type AnswerDelta = { type: "answer_delta"; text: string };
export type AnswerFinal = {
  type: "answer_final";
  text: string;
  kql_used: string | null;
  sql_used: string | null;
};
export type RunError = { type: "run_error"; code: string; message: string };
export type RunFinished = {
  type: "run_finished";
  run_id: string;
  elapsed_ms: number;
  tool_calls: number;
};
export type AgentEvent =
  | RunStarted
  | ToolCall
  | ToolResult
  | ThinkingDelta
  | AnswerDelta
  | AnswerFinal
  | RunError
  | RunFinished;

export type AskAgentRequest = {
  question: string;
  database?: string;
  include_thinking?: boolean;
};

/** A single turn in the chat transcript: one user question + the streamed agent run. */
export type ChatTurn = {
  id: string;
  question: string;
  startedAt: number;
  /** In-order trace of tool_call / tool_result entries. */
  trace: TraceEntry[];
  /** Accumulated answer text from answer_delta + answer_final. */
  answer: string;
  /** Accumulated thinking text (only populated when include_thinking is true). */
  thinking: string;
  sqlUsed: string | null;
  kqlUsed: string | null;
  model: string | null;
  elapsedMs: number | null;
  toolCalls: number | null;
  error: string | null;
  done: boolean;
};

export type TraceEntry =
  | { kind: "call"; tool: string; args: unknown; callIndex: number }
  | { kind: "result"; tool: string; result: unknown };
