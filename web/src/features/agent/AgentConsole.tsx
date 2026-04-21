import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Database, Loader2, Send, Sparkles } from "lucide-react";
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
export function AgentConsole() {
  const { endpoint, token, database } = useSession();
  const [turns, setTurns] = useState<ChatTurn[]>([]);
  const [input, setInput] = useState("");
  const [includeThinking, setIncludeThinking] = useState(false);
  const [inFlight, setInFlight] = useState(false);
  const aborterRef = useRef<AbortController | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when transcript grows.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [turns]);

  // Abort any in-flight run when the component unmounts.
  useEffect(() => {
    return () => aborterRef.current?.abort();
  }, []);

  const submit = useCallback(async () => {
    const question = input.trim();
    if (!question || inFlight) return;
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
    setInFlight(true);

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
      }
    } catch (e) {
      if ((e as Error).name === "AbortError") {
        update((t) => ({ ...t, done: true, error: t.error ?? "cancelled" }));
      } else {
        const msg = (e as Error).message || "agent request failed";
        update((t) => ({ ...t, done: true, error: msg }));
        toast.error(msg);
      }
    } finally {
      update((t) => ({ ...t, done: true }));
      setInFlight(false);
      aborterRef.current = null;
    }
  }, [input, inFlight, endpoint, token, database, includeThinking]);

  const cancel = useCallback(() => {
    aborterRef.current?.abort();
  }, []);

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submit();
    }
  };

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-2 border-b px-4 py-2 text-sm">
        <Sparkles className="h-4 w-4 text-primary" />
        <h1 className="font-semibold tracking-tight">Ask Kyma</h1>
        <div className="ml-2 flex items-center gap-1 rounded-md border bg-muted/40 px-2 py-0.5 text-xs text-muted-foreground">
          <Database className="h-3.5 w-3.5" /> {database || "—"}
        </div>
        <span className="ml-auto text-xs text-muted-foreground">
          Natural-language queries against your data.
        </span>
      </header>

      <div ref={scrollRef} className="flex-1 overflow-auto">
        <div className="mx-auto max-w-3xl space-y-4 px-4 py-4">
          {turns.length === 0 && <EmptyState />}
          {turns.map((turn) => (
            <MessageCard key={turn.id} turn={turn} />
          ))}
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
              disabled={inFlight}
              className="w-full resize-none bg-transparent px-3 py-2 text-sm outline-none placeholder:text-muted-foreground disabled:opacity-50"
            />
          </div>
          <div className="mt-2 flex items-center gap-2">
            <label className="flex items-center gap-1.5 text-xs text-muted-foreground select-none">
              <input
                type="checkbox"
                checked={includeThinking}
                onChange={(e) => setIncludeThinking(e.target.checked)}
                disabled={inFlight}
                className="h-3.5 w-3.5"
              />
              Include thinking
            </label>
            <span className="text-[11px] text-muted-foreground">
              <kbd className="rounded border bg-muted px-1">⌘↵</kbd> to send
            </span>
            <div className="ml-auto flex items-center gap-2">
              {inFlight && (
                <>
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
                  <Button variant="ghost" size="sm" onClick={cancel}>
                    Cancel
                  </Button>
                </>
              )}
              <Button
                size="sm"
                onClick={() => void submit()}
                disabled={inFlight || !input.trim()}>
                <Send className="mr-1 h-3.5 w-3.5" /> Ask
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function EmptyState() {
  return (
    <div className="mx-auto max-w-lg rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
      <Sparkles className="mx-auto mb-2 h-6 w-6 text-primary" />
      <div className="font-medium text-foreground">Ask Kyma anything</div>
      <p className="mt-1 text-xs">
        Ask in natural language — e.g. <em>"top 10 error logs in the last hour"</em> or
        <em> "describe the otel_logs schema"</em>. Kyma will pick tools, run SQL if needed,
        and cite its work.
      </p>
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
