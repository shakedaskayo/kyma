import { type ComponentProps, type FormEvent } from "react";
import { Loader2, Send, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Chat status, matching `useChat`'s `status` union. */
export type ChatStatus = "submitted" | "streaming" | "ready" | "error";

/**
 * Composer for the chat input. Mirrors AI Elements `PromptInput*`: a form
 * wrapper, a textarea that submits on plain Enter (newline on Shift+Enter),
 * a toolbar row, and a status-aware submit/stop button.
 */
export function PromptInput({
  className,
  onSubmit,
  ...props
}: Omit<ComponentProps<"form">, "onSubmit"> & {
  onSubmit: (event: FormEvent<HTMLFormElement>) => void;
}) {
  return (
    <form
      onSubmit={onSubmit}
      className={cn(
        "rounded-lg border bg-background shadow-sm focus-within:ring-2 focus-within:ring-ring",
        className,
      )}
      {...props}
    />
  );
}

export function PromptInputTextarea({
  className,
  onKeyDown,
  ...props
}: ComponentProps<"textarea">) {
  return (
    <textarea
      className={cn(
        "w-full resize-none bg-transparent px-3 py-2.5 text-sm outline-none placeholder:text-muted-foreground disabled:opacity-50",
        className,
      )}
      rows={2}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
          e.preventDefault();
          e.currentTarget.form?.requestSubmit();
        }
        onKeyDown?.(e);
      }}
      {...props}
    />
  );
}

export function PromptInputToolbar({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 border-t px-2 py-1.5",
        className,
      )}
      {...props}
    />
  );
}

export function PromptInputSubmit({
  status = "ready",
  className,
  onStop,
  disabled,
  ...props
}: Omit<ComponentProps<typeof Button>, "children"> & {
  status?: ChatStatus;
  onStop?: () => void;
}) {
  const busy = status === "submitted" || status === "streaming";

  if (busy) {
    return (
      <Button
        type="button"
        size="sm"
        variant="outline"
        className={cn("ml-auto", className)}
        onClick={onStop}
        {...props}
      >
        {status === "submitted" ? (
          <Loader2 className="mr-1 size-3.5 animate-spin" />
        ) : (
          <Square className="mr-1 size-3 fill-current" />
        )}
        Stop
      </Button>
    );
  }

  return (
    <Button
      type="submit"
      size="sm"
      className={cn("ml-auto", className)}
      disabled={disabled}
      {...props}
    >
      <Send className="mr-1 size-3.5" />
      Send
    </Button>
  );
}
