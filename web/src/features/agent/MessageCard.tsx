import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { ChevronDown, ChevronRight, Copy, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ChatTurn, TraceEntry } from "./types";

/**
 * Progressively reveal `text` at ~140 chars/sec while the run is still in
 * flight. When `done` flips true, snap to the full text immediately. This
 * smooths out Ollama's coarse chunking (the backend receives ~sentence-
 * sized deltas; without this hook the UI jumps in big visible steps).
 *
 * The `text` argument is the authoritative accumulated answer; the hook
 * never lets visible length exceed it, so if the model produces text
 * faster than 140 c/s the typewriter "catches up" mid-stream.
 */
const TYPEWRITER_CPS = 140;
function useTypewriter(text: string, done: boolean): string {
  const [visible, setVisible] = useState("");
  const textRef = useRef(text);
  textRef.current = text;

  useEffect(() => {
    if (done) {
      setVisible(textRef.current);
      return;
    }
    // Start the reveal loop. We run on requestAnimationFrame so it yields
    // cleanly to React; each tick advances `visible` by `step` characters
    // toward the current `textRef.current`, capped so we never outrun.
    let raf = 0;
    let last = performance.now();
    const tick = (now: number) => {
      const dt = now - last;
      last = now;
      const step = Math.ceil((TYPEWRITER_CPS * dt) / 1000);
      setVisible((v) => {
        const target = textRef.current;
        if (v.length >= target.length) return v;
        return target.slice(0, v.length + step);
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [done]);

  // When the incoming `text` shrinks (shouldn't happen, but defensive),
  // clamp the visible prefix.
  useEffect(() => {
    if (done) return;
    setVisible((v) => (v.length > text.length ? text : v));
  }, [text, done]);

  return done ? text : visible;
}

/**
 * A single turn in the transcript: user question at top, then the agent card
 * with reasoning trace (collapsible), markdown-ish answer, optional Copy SQL,
 * and a muted footer with stats.
 */
export function MessageCard({ turn }: { turn: ChatTurn }) {
  return (
    <div className="space-y-3">
      <UserBubble question={turn.question} />
      <AgentBubble turn={turn} />
    </div>
  );
}

function UserBubble({ question }: { question: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[80%] whitespace-pre-wrap rounded-lg bg-primary px-3 py-2 text-sm text-primary-foreground shadow-sm">
        {question}
      </div>
    </div>
  );
}

function AgentBubble({ turn }: { turn: ChatTurn }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="rounded-lg border bg-card px-4 py-3 text-card-foreground shadow-sm">
      {turn.trace.length > 0 && (
        <details
          open={open}
          onToggle={(e) => setOpen((e.currentTarget as HTMLDetailsElement).open)}
          className="mb-2.5 rounded border bg-muted/30">
          <summary className="flex cursor-pointer list-none items-center gap-1 px-2 py-1 text-xs text-muted-foreground select-none">
            {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
            Reasoning trace · {traceSummary(turn.trace)}
          </summary>
          <div className="space-y-1 px-3 py-2 font-mono text-xs">
            {turn.trace.map((entry, i) => (
              <TraceLine key={i} entry={entry} />
            ))}
          </div>
        </details>
      )}

      {turn.error ? (
        <div className="rounded border border-destructive/40 bg-destructive/5 px-2 py-1 text-xs text-destructive">
          {turn.error}
        </div>
      ) : turn.answer ? (
        <StreamedMarkdown text={turn.answer} done={turn.done} />
      ) : (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin" />
          {turn.trace.length === 0 ? "Thinking…" : "Running tools…"}
        </div>
      )}

      {turn.sqlUsed && (
        <div className="mt-3 pb-0.5">
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              navigator.clipboard.writeText(turn.sqlUsed!);
              toast.success("SQL copied to clipboard");
            }}>
            <Copy className="mr-1 h-3.5 w-3.5" /> Copy SQL
          </Button>
        </div>
      )}

      {turn.done && !turn.error && (
        <div className="mt-2.5 flex items-center gap-2 text-[11px] text-muted-foreground">
          {turn.elapsedMs != null && <span>{turn.elapsedMs}ms</span>}
          {turn.toolCalls != null && <span>· {turn.toolCalls} tool{turn.toolCalls === 1 ? "" : "s"}</span>}
          {turn.model && <span>· {turn.model}</span>}
        </div>
      )}
    </div>
  );
}

function traceSummary(trace: TraceEntry[]): string {
  const calls = trace.filter((t) => t.kind === "call").length;
  return `${calls} call${calls === 1 ? "" : "s"}`;
}

function TraceLine({ entry }: { entry: TraceEntry }) {
  if (entry.kind === "call") {
    return (
      <div>
        <span className="text-muted-foreground">→</span>{" "}
        <span className="text-foreground">{entry.tool}</span>
        <span className="text-muted-foreground">({formatArgs(entry.args)})</span>
      </div>
    );
  }
  return (
    <div className="pl-4 text-muted-foreground">
      <span className="text-foreground/80">{entry.tool}</span> → {summarizeResult(entry.result)}
    </div>
  );
}

function formatArgs(args: unknown): string {
  if (args == null) return "";
  if (typeof args !== "object") return String(args);
  try {
    const entries = Object.entries(args as Record<string, unknown>).map(([k, v]) => {
      const val =
        typeof v === "string"
          ? `"${truncate(v, 60)}"`
          : Array.isArray(v)
            ? `[${v.length}]`
            : typeof v === "object"
              ? "{…}"
              : String(v);
      return `${k}: ${val}`;
    });
    return `{${entries.join(", ")}}`;
  } catch {
    return String(args);
  }
}

function summarizeResult(result: unknown): string {
  if (result == null) return "(ok)";
  if (typeof result === "string") return truncate(result, 140);
  if (Array.isArray(result)) return `array[${result.length}]`;
  if (typeof result === "object") {
    const obj = result as Record<string, unknown>;
    if (typeof obj.error === "string") return `error: ${truncate(obj.error, 120)}`;
    if (typeof obj.rows_returned === "number") return `${obj.rows_returned} rows`;
    const keys = Object.keys(obj);
    return keys.length === 0 ? "(empty)" : `{${keys.slice(0, 4).join(", ")}${keys.length > 4 ? ", …" : ""}}`;
  }
  return String(result);
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : `${s.slice(0, n - 1)}…`;
}

/**
 * Markdown rendering with typewriter reveal while streaming. When `done`
 * flips true the full text appears immediately.
 */
function StreamedMarkdown({ text, done }: { text: string; done: boolean }) {
  const visible = useTypewriter(text, done);
  const blocks = splitByFencedCode(visible);
  return (
    <div className="space-y-2 text-[15px] leading-relaxed">
      {blocks.map((b, i) =>
        b.kind === "code" ? (
          <pre
            key={i}
            className="overflow-x-auto rounded border bg-muted/40 p-2 font-mono text-xs">
            <code>{b.text}</code>
          </pre>
        ) : (
          <Paragraphs key={i} text={b.text} />
        ),
      )}
      {!done && visible.length < text.length && (
        <span className="inline-block h-3 w-[2px] animate-pulse bg-primary align-middle" />
      )}
    </div>
  );
}

type Block = { kind: "text" | "code"; text: string; lang?: string };

function splitByFencedCode(text: string): Block[] {
  const re = /```([^\n`]*)\n([\s\S]*?)```/g;
  const out: Block[] = [];
  let cursor = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > cursor) out.push({ kind: "text", text: text.slice(cursor, m.index) });
    out.push({ kind: "code", lang: m[1]?.trim() || undefined, text: m[2] });
    cursor = m.index + m[0].length;
  }
  if (cursor < text.length) out.push({ kind: "text", text: text.slice(cursor) });
  return out;
}

function Paragraphs({ text }: { text: string }) {
  const paragraphs = text.split(/\n{2,}/).map((p) => p.trim()).filter(Boolean);
  return (
    <>
      {paragraphs.map((p, i) => (
        <p key={i} className="whitespace-pre-wrap">
          {renderInline(p)}
        </p>
      ))}
    </>
  );
}

function renderInline(text: string) {
  // Split on `inline` backticks into code spans; otherwise render as text.
  const parts = text.split(/(`[^`\n]+`)/g);
  return parts.map((p, i) =>
    p.startsWith("`") && p.endsWith("`") && p.length >= 2 ? (
      <code key={i} className="rounded bg-muted px-1 py-0.5 font-mono text-[0.85em]">
        {p.slice(1, -1)}
      </code>
    ) : (
      <span key={i}>{p}</span>
    ),
  );
}
