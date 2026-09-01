import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import { useChat } from "@ai-sdk/react";
import { getToolName, isToolUIPart, type UIMessage } from "ai";
import { Copy, Database, RotateCcw, Sparkles } from "lucide-react";
import { Button } from "../internal/ui/button";
import { usePensieveClient } from "../provider/context";
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
  const client = usePensieveClient();
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
    <div className={cn("pv-flex pv-h-full pv-flex-col", className)}>
      <header className="pv-flex pv-items-center pv-gap-2 pv-border-b pv-px-4 pv-py-2 pv-text-sm">
        <Sparkles className="pv-h-4 pv-w-4 pv-text-primary" />
        <h1 className="pv-font-semibold pv-tracking-tight">Agent</h1>
        {dbLabel && (
          <div className="pv-ml-2 pv-flex pv-items-center pv-gap-1 pv-rounded-md pv-border pv-bg-muted/40 pv-px-2 pv-py-0.5 pv-text-xs pv-text-muted-foreground">
            <Database className="pv-h-3.5 pv-w-3.5" /> {dbLabel}
          </div>
        )}
        <span className="pv-ml-auto pv-text-xs pv-text-muted-foreground">
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
            <div className="pv-flex pv-items-center pv-gap-2 pv-pl-1 pv-text-xs pv-text-muted-foreground">
              <Loader size={14} />
              Thinking…
            </div>
          )}

          {status === "error" && (
            <div className="pv-flex pv-items-center pv-gap-3 pv-rounded-md pv-border pv-border-destructive/40 pv-bg-destructive/5 pv-px-3 pv-py-2 pv-text-xs pv-text-destructive">
              <span className="pv-flex-1">
                {error?.message ?? "Agent request failed"}
              </span>
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={() => regenerate()}
              >
                <RotateCcw className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> Retry
              </Button>
            </div>
          )}
        </ConversationContent>
      </Conversation>

      <div className="pv-border-t pv-bg-background pv-px-4 pv-py-3">
        <div className="pv-mx-auto pv-max-w-3xl">
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
              <label className="pv-flex pv-items-center pv-gap-1.5 pv-text-xs pv-text-muted-foreground pv-select-none">
                <input
                  type="checkbox"
                  checked={includeThinking}
                  onChange={(e) => setIncludeThinking(e.target.checked)}
                  disabled={isBusy}
                  className="pv-h-3.5 pv-w-3.5"
                />
                Include thinking
              </label>
              <span className="pv-text-[11px] pv-text-muted-foreground">
                <kbd className="pv-rounded pv-border pv-bg-muted pv-px-1">
                  ↵
                </kbd>{" "}
                to send ·{" "}
                <kbd className="pv-rounded pv-border pv-bg-muted pv-px-1">
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
          <span className="pv-whitespace-pre-wrap">{text}</span>
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
          <div className="pv-pt-0.5">
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                navigator.clipboard.writeText(sql);
              }}
            >
              <Copy className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> Copy SQL
            </Button>
          </div>
        )}

        {(usage?.elapsed_ms != null || model) && (
          <div className="pv-flex pv-items-center pv-gap-2 pv-pt-1 pv-text-[11px] pv-text-muted-foreground">
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
    <div className="pv-flex pv-flex-col pv-items-center pv-justify-center pv-gap-4 pv-px-6 pv-py-12 pv-text-center">
      <Sparkles className="pv-h-10 pv-w-10 pv-text-muted-foreground" />
      <div>
        <h2 className="pv-text-lg pv-font-semibold">
          Ask anything about your data
        </h2>
        <p className="pv-mt-1 pv-text-sm pv-text-muted-foreground">
          The agent has access to your tables, schemas, and graph relationships.
        </p>
      </div>
      <div className="pv-grid pv-w-full pv-max-w-md pv-gap-2">
        {SUGGESTIONS.map((s) => (
          <button
            key={s}
            type="button"
            onClick={() => onSuggest(s)}
            className="pv-rounded-md pv-border pv-bg-card pv-px-3 pv-py-2 pv-text-left pv-text-sm pv-transition-colors hover:pv-bg-accent hover:pv-text-accent-foreground"
          >
            {s}
          </button>
        ))}
      </div>
    </div>
  );
}
