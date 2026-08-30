import { type ComponentProps, type FormEvent } from "react";
import { Loader2, Send, Square } from "lucide-react";
import { Button } from "../../internal/ui/button";
import { cn } from "../../internal/cn";

/** Chat status, matching `useChat`'s `status` union. */
export type ChatStatus = "submitted" | "streaming" | "ready" | "error";

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
        "pv-rounded-lg pv-border pv-bg-background pv-shadow-sm focus-within:pv-ring-2 focus-within:pv-ring-ring",
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
        "pv-w-full pv-resize-none pv-bg-transparent pv-px-3 pv-py-2.5 pv-text-sm pv-outline-none placeholder:pv-text-muted-foreground disabled:pv-opacity-50",
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
        "pv-flex pv-items-center pv-gap-2 pv-border-t pv-px-2 pv-py-1.5",
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
        className={cn("pv-ml-auto", className)}
        onClick={onStop}
        {...props}
      >
        {status === "submitted" ? (
          <Loader2 className="pv-mr-1 pv-size-3.5 pv-animate-spin" />
        ) : (
          <Square className="pv-mr-1 pv-size-3 pv-fill-current" />
        )}
        Stop
      </Button>
    );
  }

  return (
    <Button
      type="submit"
      size="sm"
      className={cn("pv-ml-auto", className)}
      disabled={disabled}
      {...props}
    >
      <Send className="pv-mr-1 pv-size-3.5" />
      Send
    </Button>
  );
}
