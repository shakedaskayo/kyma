import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useChat } from "@ai-sdk/react";
import { getToolName, isToolUIPart, type UIMessage } from "ai";
import { toast } from "sonner";
import { Copy, Database, RotateCcw, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useSession } from "@/sdk/session";
import {
  Conversation,
  ConversationContent,
} from "@/components/ai-elements/conversation";
import { Message, MessageContent } from "@/components/ai-elements/message";
import { Response } from "@/components/ai-elements/response";
import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from "@/components/ai-elements/reasoning";
import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
  type ToolState,
} from "@/components/ai-elements/tool";
import { Loader } from "@/components/ai-elements/loader";
import {
  PromptInput,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputToolbar,
} from "@/components/ai-elements/prompt-input";
import { createAgentTransport } from "./transport";

/**
 * Full chat panel for `/agent`. Backed by the AI SDK `useChat` hook talking to
 * `POST /v1/agent/ask`, which streams the AI-SDK UI Message Stream protocol.
 * Multi-turn context is preserved by capturing the conversation `session_id`
 * (surfaced as a `data-session` part) and echoing it back on the next turn.
 *
 * State is in-memory; navigating away and back resets the transcript.
 */

const SUGGESTIONS = [
  "What databases do we have?",
  "Show me the schema of the largest table",
  "Find any references to 'admin' across our data",
] as const;

export function AgentConsole() {
  const { endpoint, token, database } = useSession();
  const [input, setInput] = useState("");
  const [includeThinking, setIncludeThinking] = useState(false);

  // Per-turn request options read at send time (so changing the database or
  // the thinking toggle mid-conversation is picked up without rebuilding the
  // transport). `sessionId` is the server's conversation id for `--resume`.
  const sessionIdRef = useRef<string | null>(null);
  const optsRef = useRef({ database, includeThinking });
  optsRef.current = { database, includeThinking };

  const transport = useMemo(
    () =>
      createAgentTransport(endpoint, token, () => ({
        database: optsRef.current.database || undefined,
        include_thinking: optsRef.current.includeThinking,
        session_id: sessionIdRef.current ?? undefined,
      })),
    [endpoint, token],
  );

  const { messages, sendMessage, status, stop, error, regenerate } = useChat({
    transport,
  });

  // Capture the conversation session id from any `data-session` part so the
  // next turn resumes the same conversation.
  useEffect(() => {
    for (let i = messages.length - 1; i >= 0; i--) {
      const part = messages[i].parts.find((p) => p.type === "data-session");
      if (part) {
        const sid = (part as { data?: { sessionId?: string } }).data?.sessionId;
        if (typeof sid === "string" && sid.length > 0) {
          sessionIdRef.current = sid;
        }
        break;
      }
    }
  }, [messages]);

  const isBusy = status === "submitted" || status === "streaming";

  const submit = (e?: FormEvent<HTMLFormElement>) => {
    e?.preventDefault();
    const question = input.trim();
    if (!question || isBusy) return;
    if (!endpoint) {
      toast.error("Configure server endpoint in Settings");
      return;
    }
    sendMessage({ text: question });
    setInput("");
  };

  const lastMessage = messages[messages.length - 1];
  const awaitingFirstToken =
    isBusy &&
    (lastMessage?.role !== "assistant" ||
      !lastMessage.parts.some(
        (p) =>
          (p.type === "text" && (p as { text: string }).text.length > 0) ||
          p.type === "reasoning" ||
          isToolUIPart(p),
      ));

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-2 border-b px-4 py-2 text-sm">
        <Sparkles className="h-4 w-4 text-primary" />
        <h1 className="font-semibold tracking-tight">Agent</h1>
        <div className="ml-2 flex items-center gap-1 rounded-md border bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
          <Database className="h-3.5 w-3.5" /> {database || "—"}
        </div>
        <span className="ml-auto text-xs text-muted-foreground">
          Natural-language queries against your data.
        </span>
      </header>

      <Conversation>
        <ConversationContent>
          {messages.length === 0 && (
            <EmptyState onSuggest={(s) => setInput(s)} />
          )}

          {messages.map((message) => (
            <MessageView key={message.id} message={message} />
          ))}

          {awaitingFirstToken && (
            <div className="flex items-center gap-2 pl-1 text-xs text-muted-foreground">
              <Loader size={14} />
              Thinking…
            </div>
          )}

          {status === "error" && (
            <div className="flex items-center gap-3 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              <span className="flex-1">{error?.message ?? "Agent request failed"}</span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => regenerate()}
              >
                <RotateCcw className="mr-1 h-3.5 w-3.5" /> Retry
              </Button>
            </div>
          )}
        </ConversationContent>
      </Conversation>

      <div className="border-t bg-background px-4 py-3">
        <div className="mx-auto max-w-3xl">
          <PromptInput onSubmit={submit}>
            <PromptInputTextarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder={`Ask a question about ${database || "your data"}…`}
              disabled={isBusy}
            />
            <PromptInputToolbar>
              <label className="flex items-center gap-1.5 text-xs text-muted-foreground select-none">
                <input
                  type="checkbox"
                  checked={includeThinking}
                  onChange={(e) => setIncludeThinking(e.target.checked)}
                  disabled={isBusy}
                  className="h-3.5 w-3.5"
                />
                Include thinking
              </label>
              <span className="text-[11px] text-muted-foreground">
                <kbd className="rounded border bg-muted px-1">↵</kbd> to send ·{" "}
                <kbd className="rounded border bg-muted px-1">⇧↵</kbd> for newline
              </span>
              <PromptInputSubmit
                status={status}
                onStop={stop}
                disabled={!input.trim()}
              />
            </PromptInputToolbar>
          </PromptInput>
        </div>
      </div>
    </div>
  );
}

/** Render one message: a user bubble or an assistant card with its parts. */
function MessageView({ message }: { message: UIMessage }) {
  if (message.role === "user") {
    const text = message.parts
      .filter((p) => p.type === "text")
      .map((p) => (p as { text: string }).text)
      .join("");
    return (
      <Message from="user">
        <MessageContent>
          <span className="whitespace-pre-wrap">{text}</span>
        </MessageContent>
      </Message>
    );
  }

  // Assistant: surface SQL/usage data parts in the footer.
  const sql = findDataPart<{ sql: string }>(message, "data-sql")?.sql;
  const usage = findDataPart<{ elapsed_ms?: number; total_cost_usd?: number | null }>(
    message,
    "data-usage",
  );
  const model = findDataPart<{ model: string }>(message, "data-model")?.model;

  return (
    <Message from="assistant">
      <MessageContent>
        {message.parts.map((part, i) => {
          if (part.type === "reasoning") {
            const text = (part as { text: string }).text;
            const streaming = (part as { state?: string }).state === "streaming";
            if (!text) return null;
            return (
              <Reasoning key={i} isStreaming={streaming}>
                <ReasoningTrigger />
                <ReasoningContent>{text}</ReasoningContent>
              </Reasoning>
            );
          }
          if (part.type === "text") {
            const text = (part as { text: string }).text;
            if (!text) return null;
            return <Response key={i}>{text}</Response>;
          }
          if (isToolUIPart(part)) {
            const state = part.state as ToolState;
            return (
              <Tool key={i} defaultOpen={state === "output-error"}>
                <ToolHeader type={getToolName(part)} state={state} />
                <ToolContent>
                  <ToolInput input={part.input} />
                  {(state === "output-available" || state === "output-error") && (
                    <ToolOutput
                      output={part.output}
                      errorText={part.errorText}
                    />
                  )}
                </ToolContent>
              </Tool>
            );
          }
          return null;
        })}

        {sql && (
          <div className="pt-0.5">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard.writeText(sql);
                toast.success("SQL copied to clipboard");
              }}
            >
              <Copy className="mr-1 h-3.5 w-3.5" /> Copy SQL
            </Button>
          </div>
        )}

        {(usage?.elapsed_ms != null || model) && (
          <div className="flex items-center gap-2 pt-1 text-[11px] text-muted-foreground">
            {usage?.elapsed_ms != null && <span>{usage.elapsed_ms}ms</span>}
            {usage?.total_cost_usd != null && (
              <span>· ${usage.total_cost_usd.toFixed(4)}</span>
            )}
            {model && <span>· {model}</span>}
          </div>
        )}
      </MessageContent>
    </Message>
  );
}

function findDataPart<T>(message: UIMessage, type: string): T | undefined {
  const part = message.parts.find((p) => p.type === type);
  return part ? ((part as { data?: T }).data as T) : undefined;
}

function EmptyState({ onSuggest }: { onSuggest: (s: string) => void }) {
  return (
    <div className="flex flex-col items-center justify-center gap-4 px-6 py-12 text-center">
      <Sparkles className="h-10 w-10 text-muted-foreground" />
      <div>
        <h2 className="text-lg font-semibold">Ask anything about your data</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          The agent has access to your tables, schemas, and graph relationships.
        </p>
      </div>
      <div className="grid w-full max-w-md gap-2">
        {SUGGESTIONS.map((s) => (
          <button
            key={s}
            type="button"
            onClick={() => onSuggest(s)}
            className="rounded-md border bg-card px-3 py-2 text-left text-sm transition-colors hover:bg-accent hover:text-accent-foreground"
          >
            {s}
          </button>
        ))}
      </div>
    </div>
  );
}
