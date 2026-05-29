import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Database, Send, Sparkles, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useSession } from "@/sdk/session";
import { runAgent } from "./sse";
import type { AgentEvent, ChatTurn } from "./types";
import { MessageCard } from "./MessageCard";

/**
 * Full chat panel for `/agent`. State is kept in-memory; navigating away and
 * back resets the transcript (matches the spec — "no persistence between route
 * visits").
 */

const SUGGESTIONS = [
  "What databases do we have?",
  "Show me the schema of the largest table",
  "Find any references to 'admin' across our data",
] as const;

export function AgentConsole() {
  const { endpoint, token, database } = useSession();
  const [turns, setTurns] = useState<ChatTurn[]>([]);
  const [input, setInput] = useState("");
  const [includeThinking, setIncludeThinking] = useState(false);
  // "idle" | "streaming" | "error" | "done"
  const [status, setStatus] = useState<"idle" | "streaming" | "error" | "done">("idle");
  const aborterRef = useRef<AbortController | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const bottomRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when transcript grows, but only when the user
  // hasn't manually scrolled up (i.e. bottomRef is within ~100px of the
  // scroll container's bottom edge).
  const scrollToBottomIfNear = useCallback(() => {
    const container = scrollRef.current;
    const bottom = bottomRef.current;
    if (!container || !bottom) return;
    const distanceFromBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight;
    if (distanceFromBottom <= 100) {
      bottom.scrollIntoView({ block: "end", behavior: "smooth" });
    }
  }, []);

  useEffect(() => {
    scrollToBottomIfNear();
  }, [turns, scrollToBottomIfNear]);

  // Abort any in-flight run when the component unmounts.
  useEffect(() => {
    return () => aborterRef.current?.abort();
  }, []);

  const submit = useCallback(async () => {
    const question = input.trim();
    if (!question || status === "streaming") return;
    if (!endpoint) {
      toast.error("Configure server endpoint in Settings");
      return;
    }

    const turn: ChatTurn = {
      id: crypto.randomUUID(),
      question,
      startedAt: Date.now(),
      trace: [],
      answer: "",
      thinking: "",
      sqlUsed: null,
      kqlUsed: null,
      model: null,
      elapsedMs: null,
      toolCalls: null,
      error: null,
      done: false,
    };
    setTurns((t) => [...t, turn]);
    setInput("");
    setStatus("streaming");

    const update = (fn: (t: ChatTurn) => ChatTurn) =>
      setTurns((list) => list.map((t) => (t.id === turn.id ? fn(t) : t)));

    const ctl = new AbortController();
    aborterRef.current = ctl;
    try {
      for await (const ev of runAgent(
        endpoint,
        token || undefined,
        { question, database: database || undefined, include_thinking: includeThinking },
        ctl.signal,
      )) {
        update((t) => applyEvent(t, ev));
        scrollToBottomIfNear();
      }
      setStatus("done");
    } catch (e) {
      if ((e as Error).name === "AbortError") {
        update((t) => ({ ...t, done: true, error: t.error ?? "cancelled" }));
        setStatus("done");
      } else {
        const msg = (e as Error).message || "agent request failed";
        update((t) => ({ ...t, done: true, error: msg }));
        toast.error(msg);
        setStatus("error");
      }
    } finally {
      update((t) => ({ ...t, done: true }));
      aborterRef.current = null;
    }
  }, [input, status, endpoint, token, database, includeThinking, scrollToBottomIfNear]);

  const stop = useCallback(() => {
    aborterRef.current?.abort();
  }, []);

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submit();
    }
  };

  const isStreaming = status === "streaming";

  // Determine if the last turn has received any answer text yet.
  // We check turn.answer.length since that's populated by answer_delta / answer_final events.
  const lastTurn = turns.length > 0 ? turns[turns.length - 1] : null;
  const lastTurnHasAnswer = lastTurn != null && lastTurn.answer.length > 0;

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

      <div ref={scrollRef} className="flex-1 overflow-auto">
        <div className="mx-auto max-w-3xl space-y-4 px-4 py-4">
          {turns.length === 0 && (
            <EmptyState onSuggest={(s) => setInput(s)} />
          )}
          {turns.map((turn, i) => {
            const isLast = i === turns.length - 1;
            return (
              <div key={turn.id}>
                <MessageCard turn={turn} />
                {isLast && isStreaming && !lastTurnHasAnswer && (
                  <ThinkingIndicator />
                )}
              </div>
            );
          })}
          <div ref={bottomRef} />
        </div>
      </div>

      <div className="border-t bg-background px-4 py-3">
        <div className="mx-auto max-w-3xl">
          <div className="relative rounded-md border bg-background focus-within:ring-2 focus-within:ring-ring">
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder={`Ask a question about ${database || "your data"}…`}
              rows={3}
              disabled={isStreaming}
              className="w-full resize-none bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground disabled:opacity-50"
            />
          </div>
          <div className="mt-2 flex items-center gap-2">
            <label className="flex items-center gap-1.5 text-xs text-muted-foreground select-none">
              <input
                type="checkbox"
                checked={includeThinking}
                onChange={(e) => setIncludeThinking(e.target.checked)}
                disabled={isStreaming}
                className="h-3.5 w-3.5"
              />
              Include thinking
            </label>
            <span className="text-[11px] text-muted-foreground">
              <kbd className="rounded border bg-muted px-1">⌘↵</kbd>{" "}
              to send &middot;{" "}
              <kbd className="rounded border bg-muted px-1">⇧↵</kbd>{" "}
              for newline
            </span>
            <div className="ml-auto flex items-center gap-2">
              {isStreaming ? (
                <Button type="button" variant="outline" size="sm" onClick={stop}>
                  <Square className="mr-1 h-3 w-3 fill-current" /> Stop
                </Button>
              ) : (
                <Button
                  type="submit"
                  size="sm"
                  onClick={() => void submit()}
                  disabled={!input.trim()}>
                  <Send className="mr-1 h-3.5 w-3.5" /> Send
                </Button>
              )}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
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
            className="rounded-md border bg-card px-3 py-2 text-left text-sm hover:bg-accent hover:text-accent-foreground transition-colors"
          >
            {s}
          </button>
        ))}
      </div>
    </div>
  );
}

function ThinkingIndicator() {
  return (
    <div className="flex items-center gap-1.5 py-2 pl-1 text-xs text-muted-foreground">
      <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-muted-foreground/70" />
      <span
        className="h-1.5 w-1.5 animate-pulse rounded-full bg-muted-foreground/70"
        style={{ animationDelay: "150ms" }}
      />
      <span
        className="h-1.5 w-1.5 animate-pulse rounded-full bg-muted-foreground/70"
        style={{ animationDelay: "300ms" }}
      />
      <span className="ml-1">Thinking…</span>
    </div>
  );
}

/** Apply a single SSE event to an in-flight ChatTurn. Pure function for easy testing. */
export function applyEvent(t: ChatTurn, ev: AgentEvent): ChatTurn {
  switch (ev.type) {
    case "run_started":
      return { ...t, model: ev.model };
    case "tool_call":
      return {
        ...t,
        trace: [...t.trace, { kind: "call", tool: ev.tool, args: ev.args, callIndex: ev.call_index }],
      };
    case "tool_result":
      return {
        ...t,
        trace: [...t.trace, { kind: "result", tool: ev.tool, result: ev.result }],
      };
    case "thinking_delta":
      return { ...t, thinking: t.thinking + ev.text };
    case "answer_delta":
      return { ...t, answer: t.answer + ev.text };
    case "answer_final":
      return {
        ...t,
        // Prefer the final, authoritative text over accumulated deltas.
        answer: ev.text || t.answer,
        sqlUsed: ev.sql_used,
        kqlUsed: ev.kql_used,
      };
    case "run_error":
      return { ...t, error: `${ev.code}: ${ev.message}`, done: true };
    case "run_finished":
      return { ...t, elapsedMs: ev.elapsed_ms, toolCalls: ev.tool_calls, done: true };
    default:
      return t;
  }
}
