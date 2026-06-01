import { useEffect, useMemo, useRef, useState } from "react";
import { useChat } from "@ai-sdk/react";
import { getToolName, isToolUIPart, type UIMessage } from "ai";
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
import { createAgentTransport } from "./transport";

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
 * Compact inline Ask AI dialog used from the Explore page. Runs a one-shot
 * agent turn via `useChat`; shows a live reasoning blip while running, then
 * either injects the generated SQL (from a `data-sql` part) into the editor or
 * surfaces the answer text.
 */
export function AskAIDialog({ open, onOpenChange, onSql, onNoSql }: Props) {
  const { endpoint, token, database } = useSession();
  const [question, setQuestion] = useState("");
  const [error, setError] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const dbRef = useRef(database);
  dbRef.current = database;

  const transport = useMemo(
    () =>
      createAgentTransport(endpoint, token, () => ({
        database: dbRef.current || undefined,
      })),
    [endpoint, token],
  );

  const {
    messages,
    sendMessage,
    status,
    stop,
    setMessages,
    error: chatError,
  } = useChat({ transport });

  const inFlight = status === "submitted" || status === "streaming";

  // Reset transient state whenever the dialog opens or closes.
  useEffect(() => {
    if (!open) {
      stop();
      pendingRef.current = false;
      setError(null);
    } else {
      setQuestion("");
      setError(null);
      setMessages([]);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Finalize once the turn completes: inject SQL or surface the answer.
  useEffect(() => {
    if (!pendingRef.current) return;
    if (status === "ready") {
      pendingRef.current = false;
      const assistant = [...messages]
        .reverse()
        .find((m) => m.role === "assistant");
      const sql = assistant ? findDataSql(assistant) : null;
      const answer = assistant ? assistantText(assistant) : "";
      if (sql) {
        onSql(sql);
      } else {
        onNoSql(answer || "Agent returned no answer.");
      }
      onOpenChange(false);
    } else if (status === "error") {
      pendingRef.current = false;
      const msg = chatError?.message ?? "agent request failed";
      setError(msg);
      toast.error(msg);
    }
  }, [status, messages, chatError, onSql, onNoSql, onOpenChange]);

  const submit = () => {
    const q = question.trim();
    if (!q || inFlight) return;
    if (!endpoint) {
      toast.error("Configure server endpoint in Settings");
      return;
    }
    setError(null);
    setMessages([]);
    pendingRef.current = true;
    sendMessage({ text: q });
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    }
  };

  // Live progress blip derived from the in-flight assistant message's tools.
  const assistant = [...messages].reverse().find((m) => m.role === "assistant");
  const toolParts = assistant ? assistant.parts.filter(isToolUIPart) : [];
  const currentToolName = toolParts.length
    ? getToolName(toolParts[toolParts.length - 1])
    : null;
  const toolsRun = toolParts.filter(
    (p) => p.state === "output-available" || p.state === "output-error",
  ).length;

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
                {currentToolName ? (
                  <>→ <span className="font-mono">{currentToolName}</span></>
                ) : (
                  "Thinking…"
                )}
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
          <Button onClick={submit} disabled={inFlight || !question.trim()}>
            {inFlight ? (
              <><Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" /> Running</>
            ) : (
              "Ask"
            )}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

function assistantText(message: UIMessage): string {
  return message.parts
    .filter((p) => p.type === "text")
    .map((p) => (p as { text: string }).text)
    .join("");
}

function findDataSql(message: UIMessage): string | null {
  const part = message.parts.find((p) => p.type === "data-sql");
  const sql = part ? (part as { data?: { sql?: string } }).data?.sql : undefined;
  return typeof sql === "string" && sql.length > 0 ? sql : null;
}
