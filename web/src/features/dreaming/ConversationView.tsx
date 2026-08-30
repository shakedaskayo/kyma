import { AlertTriangle, MessagesSquare, Sparkles } from "lucide-react";
import { EmptyState } from "@/components/ui/empty-state";
import {
  Loader,
  Message,
  MessageContent,
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
  Response,
  Tool,
  ToolContent,
  ToolHeader,
  ToolInput,
  ToolOutput,
} from "@pensieve-ai/react/agent";
import { cn } from "@/lib/utils";
import type { ConversationPart } from "./traceToParts";

/**
 * Static render of a folded agent trace as a conversation: the question as a
 * user message, then a single assistant message holding the ordered parts.
 * Mirrors AgentConsole's Message/Response/Reasoning/Tool composition. The final
 * text block is wrapped with a violet "Summary" accent. `live` appends a loader.
 */
export function ConversationView({
  question,
  parts,
  live,
}: {
  question: string;
  parts: ConversationPart[];
  live?: boolean;
}) {
  if (!question && parts.length === 0 && !live) {
    return (
      <EmptyState
        icon={MessagesSquare}
        title="No conversation recorded"
        description="This run did not record an agent conversation trace."
      />
    );
  }

  // Index of the last text part — gets the Summary accent treatment.
  let lastTextIndex = -1;
  for (let i = parts.length - 1; i >= 0; i--) {
    if (parts[i].kind === "text") {
      lastTextIndex = i;
      break;
    }
  }

  return (
    <div className="space-y-4">
      {question && (
        <Message from="user">
          <MessageContent>
            <span className="whitespace-pre-wrap">{question}</span>
          </MessageContent>
        </Message>
      )}

      <Message from="assistant">
        <MessageContent>
          {parts.map((part, i) => {
            if (part.kind === "reasoning") {
              if (!part.text) return null;
              return (
                <Reasoning key={i} defaultOpen={false}>
                  <ReasoningTrigger />
                  <ReasoningContent>{part.text}</ReasoningContent>
                </Reasoning>
              );
            }
            if (part.kind === "text") {
              if (!part.text) return null;
              if (i === lastTextIndex) {
                return (
                  <div
                    key={i}
                    className="rounded-md border-l-2 border-l-violet-500 bg-violet-500/[0.04] py-1.5 pl-3 pr-1"
                  >
                    <div className="mb-1 flex items-center gap-1.5 text-2xs font-medium uppercase tracking-wide text-violet-500">
                      <Sparkles className="h-3 w-3" /> Summary
                    </div>
                    <Response>{part.text}</Response>
                  </div>
                );
              }
              return <Response key={i}>{part.text}</Response>;
            }
            if (part.kind === "tool") {
              return (
                <Tool key={i} defaultOpen={part.state === "output-error"}>
                  <ToolHeader type={part.tool} state={part.state} />
                  <ToolContent>
                    <ToolInput input={part.input} />
                    {(part.state === "output-available" || part.state === "output-error") && (
                      <ToolOutput output={part.output} errorText={part.errorText} />
                    )}
                  </ToolContent>
                </Tool>
              );
            }
            if (part.kind === "error") {
              return (
                <div
                  key={i}
                  className={cn(
                    "flex items-start gap-2 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive",
                  )}
                >
                  <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <div className="min-w-0">
                    {part.code && <span className="font-mono">{part.code}: </span>}
                    <span className="break-words">{part.message}</span>
                  </div>
                </div>
              );
            }
            return null;
          })}

          {live && (
            <div className="flex items-center gap-2 pt-1 text-xs text-muted-foreground">
              <Loader size={14} /> Dreaming…
            </div>
          )}
        </MessageContent>
      </Message>
    </div>
  );
}
