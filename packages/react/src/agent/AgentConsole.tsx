import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useChat } from "@ai-sdk/react";
import { getToolName, isToolUIPart, type UIMessage } from "ai";
import { Copy, Database, RotateCcw, Sparkles } from "lucide-react";
import { Button } from "../internal/ui/button";
import { useKymaClient } from "../provider/context";
import {
  Conversation,
  ConversationContent,
} from "./elements/conversation";
import { Message, MessageContent } from "./elements/message";
import { Response } from "./elements/response";
import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from "./elements/reasoning";
import {
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
  type ToolState,
} from "./elements/tool";
import { Loader } from "./elements/loader";
import {
  PromptInput,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputToolbar,
} from "./elements/prompt-input";
import { createAgentTransport } from "./transport";
import { cn } from "../internal/cn";

export interface AgentConsoleProps {
  /** Override the active database for agent requests. */
  database?: string;
  /** Placeholder text for the input textarea. */
  placeholder?: string;
  /** Extra CSS class for the container. */
  className?: string;
  /**
   * Called when an assistant message finishes streaming.
   * Fires once per completed turn with the full assembled text.
   */
  onAssistantMessage?: (m: { role: string; text: string }) => void;
}

const SUGGESTIONS = [
  "What databases do we have?",
  "Show me the schema of the largest table",
  "Find any references to 'admin' across our data",
] as const;

/**
 * Full chat panel backed by the AI SDK `useChat` hook talking to
 * `POST /v1/agent/ask`. Auth is routed through `client.transport.request()`
 * so token refresh/injection is automatic.
 *
 * Multi-turn context is preserved by capturing the conversation `session_id`
 * (surfaced as a `data-session` part) and echoing it on the next turn.
 * State is in-memory; unmounting resets the transcript.
 */
export function AgentConsole({ database, placeholder, className, onAssistantMessage }: AgentConsoleProps) {
  const client = useKymaClient();
  const [input, setInput] = useState("");
  const [includeThinking, setIncludeThinking] = useState(false);

  const sessionIdRef = useRef<string | null>(null);
  const optsRef = useRef({ database, includeThinking });
  optsRef.current = { database, includeThinking };

  const transport = useMemo(
    () =>
      createAgentTransport(client, () => ({
        database: optsRef.current.database || undefined,
        include_thinking: optsRef.current.includeThinking,
        session_id: sessionIdRef.current ?? undefined,
      })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [client],
  );

  const onAssistantMessageRef = useRef(onAssistantMessage);
  onAssistantMessageRef.current = onAssistantMessage;

  const { messages, sendMessage, status, stop, error, regenerate } = useChat({
    transport,
    onFinish: ({ message }) => {
      if (message.role !== "assistant") return;
      const text = message.parts
        .filter((p) => p.type === "text")
        .map((p) => (p as { text: string }).text)
        .join("");
      onAssistantMessageRef.current?.({ role: "assistant", text });
    },
  });

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

  const dbLabel = database || client.transport.database;

  return (
    <div className={cn("ky-flex ky-h-full ky-flex-col", className)}>
      <header className="ky-flex ky-items-center ky-gap-2 ky-border-b ky-px-4 ky-py-2 ky-text-sm">
        <Sparkles className="ky-h-4 ky-w-4 ky-text-primary" />
        <h1 className="ky-font-semibold ky-tracking-tight">Agent</h1>
        {dbLabel && (
          <div className="ky-ml-2 ky-flex ky-items-center ky-gap-1 ky-rounded-md ky-border ky-bg-muted/40 ky-px-2 ky-py-0.5 ky-text-xs ky-text-muted-foreground">
            <Database className="ky-h-3.5 ky-w-3.5" /> {dbLabel}
          </div>
        )}
        <span className="ky-ml-auto ky-text-xs ky-text-muted-foreground">
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
            <div className="ky-flex ky-items-center ky-gap-2 ky-pl-1 ky-text-xs ky-text-muted-foreground">
              <Loader size={14} />
              Thinking…
            </div>
          )}

          {status === "error" && (
            <div className="ky-flex ky-items-center ky-gap-3 ky-rounded-md ky-border ky-border-destructive/40 ky-bg-destructive/5 ky-px-3 ky-py-2 ky-text-xs ky-text-destructive">
              <span className="ky-flex-1">
                {error?.message ?? "Agent request failed"}
              </span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => regenerate()}
              >
                <RotateCcw className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Retry
              </Button>
            </div>
          )}
        </ConversationContent>
      </Conversation>

      <div className="ky-border-t ky-bg-background ky-px-4 ky-py-3">
        <div className="ky-mx-auto ky-max-w-3xl">
          <PromptInput onSubmit={submit}>
            <PromptInputTextarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder={
                placeholder ??
                `Ask a question about ${dbLabel || "your data"}…`
              }
              disabled={isBusy}
            />
            <PromptInputToolbar>
              <label className="ky-flex ky-items-center ky-gap-1.5 ky-text-xs ky-text-muted-foreground ky-select-none">
                <input
                  type="checkbox"
                  checked={includeThinking}
                  onChange={(e) => setIncludeThinking(e.target.checked)}
                  disabled={isBusy}
                  className="ky-h-3.5 ky-w-3.5"
                />
                Include thinking
              </label>
              <span className="ky-text-[11px] ky-text-muted-foreground">
                <kbd className="ky-rounded ky-border ky-bg-muted ky-px-1">
                  ↵
                </kbd>{" "}
                to send ·{" "}
                <kbd className="ky-rounded ky-border ky-bg-muted ky-px-1">
                  ⇧↵
                </kbd>{" "}
                for newline
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

function MessageView({ message }: { message: UIMessage }) {
  if (message.role === "user") {
    const text = message.parts
      .filter((p) => p.type === "text")
      .map((p) => (p as { text: string }).text)
      .join("");
    return (
      <Message from="user">
        <MessageContent>
          <span className="ky-whitespace-pre-wrap">{text}</span>
        </MessageContent>
      </Message>
    );
  }

  const sql = findDataPart<{ sql: string }>(message, "data-sql")?.sql;
  const usage = findDataPart<{
    elapsed_ms?: number;
    total_cost_usd?: number | null;
  }>(message, "data-usage");
  const model = findDataPart<{ model: string }>(message, "data-model")?.model;

  return (
    <Message from="assistant">
      <MessageContent>
        {message.parts.map((part, i) => {
          if (part.type === "reasoning") {
            const text = (part as { text: string }).text;
            const streaming =
              (part as { state?: string }).state === "streaming";
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
                  {(state === "output-available" ||
                    state === "output-error") && (
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
          <div className="ky-pt-0.5">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard.writeText(sql);
              }}
            >
              <Copy className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Copy SQL
            </Button>
          </div>
        )}

        {(usage?.elapsed_ms != null || model) && (
          <div className="ky-flex ky-items-center ky-gap-2 ky-pt-1 ky-text-[11px] ky-text-muted-foreground">
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
    <div className="ky-flex ky-flex-col ky-items-center ky-justify-center ky-gap-4 ky-px-6 ky-py-12 ky-text-center">
      <Sparkles className="ky-h-10 ky-w-10 ky-text-muted-foreground" />
      <div>
        <h2 className="ky-text-lg ky-font-semibold">
          Ask anything about your data
        </h2>
        <p className="ky-mt-1 ky-text-sm ky-text-muted-foreground">
          The agent has access to your tables, schemas, and graph relationships.
        </p>
      </div>
      <div className="ky-grid ky-w-full ky-max-w-md ky-gap-2">
        {SUGGESTIONS.map((s) => (
          <button
            key={s}
            type="button"
            onClick={() => onSuggest(s)}
            className="ky-rounded-md ky-border ky-bg-card ky-px-3 ky-py-2 ky-text-left ky-text-sm ky-transition-colors hover:ky-bg-accent hover:ky-text-accent-foreground"
          >
            {s}
          </button>
        ))}
      </div>
    </div>
  );
}
