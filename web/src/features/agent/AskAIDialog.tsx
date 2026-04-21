import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { Loader2, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useSession } from "@/sdk/session";
import { runAgent } from "./sse";
import type { AgentEvent, TraceEntry } from "./types";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Called with the SQL the agent ran. The parent decides whether to inject
   * into the editor or fall back to clipboard. */
  onSql: (sql: string) => void;
  /** Called when the agent finished without running any SQL — parent shows
   * the raw answer as a toast. */
  onNoSql: (answer: string) => void;
};

/**
 * Compact inline Ask AI dialog used from the Explore page. Shows a live
 * reasoning blip while the agent is running; on `answer_final`, either
 * injects the generated SQL into the editor or surfaces the answer text.
 */
export function AskAIDialog({ open, onOpenChange, onSql, onNoSql }: Props) {
  const { endpoint, token, database } = useSession();
  const [question, setQuestion] = useState("");
  const [inFlight, setInFlight] = useState(false);
  const [currentTool, setCurrentTool] = useState<TraceEntry | null>(null);
  const [toolsRun, setToolsRun] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const aborterRef = useRef<AbortController | null>(null);

  // Reset transient state whenever the dialog opens or closes.
  useEffect(() => {
    if (!open) {
      aborterRef.current?.abort();
      aborterRef.current = null;
      setInFlight(false);
      setCurrentTool(null);
      setToolsRun(0);
      setError(null);
    } else {
      setQuestion("");
    }
  }, [open]);

  const submit = useCallback(async () => {
    const q = question.trim();
    if (!q || inFlight) return;
    if (!endpoint) {
      toast.error("Configure server endpoint in Settings");
      return;
    }
    setError(null);
    setCurrentTool(null);
    setToolsRun(0);
    setInFlight(true);
    const ctl = new AbortController();
    aborterRef.current = ctl;

    let sql: string | null = null;
    let answer = "";
    try {
      for await (const ev of runAgent(
        endpoint,
        token || undefined,
        { question: q, database: database || undefined },
        ctl.signal,
      )) {
        applyDialogEvent(ev, (patch) => {
          if (patch.currentTool !== undefined) setCurrentTool(patch.currentTool);
          if (patch.incrementToolsRun) setToolsRun((n) => n + 1);
          if (patch.error) setError(patch.error);
        });
        if (ev.type === "answer_final") {
          sql = ev.sql_used;
          answer = ev.text;
        }
        if (ev.type === "run_error") {
          throw new Error(`${ev.code}: ${ev.message}`);
        }
      }
    } catch (e) {
      if ((e as Error).name !== "AbortError") {
        const msg = (e as Error).message || "agent request failed";
        setError(msg);
        toast.error(msg);
      }
      setInFlight(false);
      return;
    }
    setInFlight(false);
    aborterRef.current = null;

    if (sql) {
      onSql(sql);
      onOpenChange(false);
    } else {
      onNoSql(answer || "Agent returned no answer.");
      onOpenChange(false);
    }
  }, [question, inFlight, endpoint, token, database, onSql, onNoSql, onOpenChange]);

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submit();
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-base">
            <Sparkles className="h-4 w-4 text-primary" /> Ask AI
          </DialogTitle>
          <DialogDescription className="text-xs">
            Describe the query in natural language. Kyma will generate SQL and inject it
            into the editor.
          </DialogDescription>
        </DialogHeader>

        <textarea
          autoFocus
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="What do you want to know?"
          rows={4}
          disabled={inFlight}
          className="w-full resize-none rounded-md border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
        />

        {inFlight && (
          <div className="flex items-start gap-2 rounded-md border bg-muted/30 px-3 py-2 text-xs">
            <Loader2 className="mt-0.5 h-3.5 w-3.5 animate-spin text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <div className="font-medium text-foreground">
                {currentTool?.kind === "call"
                  ? <>→ <span className="font-mono">{currentTool.tool}</span></>
                  : "Thinking…"}
              </div>
              <div className="text-muted-foreground">
                {toolsRun > 0 && `${toolsRun} tool${toolsRun === 1 ? "" : "s"} so far`}
              </div>
            </div>
          </div>
        )}

        {error && (
          <div className="rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            {error}
          </div>
        )}

        <div className="flex justify-end gap-2 pt-2">
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={inFlight && !error}>
            Cancel
          </Button>
          <Button onClick={() => void submit()} disabled={inFlight || !question.trim()}>
            {inFlight ? <><Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> Running</> : "Ask"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

type DialogPatch = {
  currentTool?: TraceEntry | null;
  incrementToolsRun?: boolean;
  error?: string;
};

function applyDialogEvent(ev: AgentEvent, patch: (p: DialogPatch) => void) {
  switch (ev.type) {
    case "tool_call":
      patch({ currentTool: { kind: "call", tool: ev.tool, args: ev.args, callIndex: ev.call_index } });
      break;
    case "tool_result":
      patch({ currentTool: { kind: "result", tool: ev.tool, result: ev.result }, incrementToolsRun: true });
      break;
    case "run_error":
      patch({ error: `${ev.code}: ${ev.message}` });
      break;
  }
}
