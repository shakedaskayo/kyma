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
        "ky-rounded-lg ky-border ky-bg-background ky-shadow-sm focus-within:ky-ring-2 focus-within:ky-ring-ring",
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
        "ky-w-full ky-resize-none ky-bg-transparent ky-px-3 ky-py-2.5 ky-text-sm ky-outline-none placeholder:ky-text-muted-foreground disabled:ky-opacity-50",
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
        "ky-flex ky-items-center ky-gap-2 ky-border-t ky-px-2 ky-py-1.5",
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
        className={cn("ky-ml-auto", className)}
        onClick={onStop}
        {...props}
      >
        {status === "submitted" ? (
          <Loader2 className="ky-mr-1 ky-size-3.5 ky-animate-spin" />
        ) : (
          <Square className="ky-mr-1 ky-size-3 ky-fill-current" />
        )}
        Stop
      </Button>
    );
  }

  return (
    <Button
      type="submit"
      size="sm"
      className={cn("ky-ml-auto", className)}
      disabled={disabled}
      {...props}
    >
      <Send className="ky-mr-1 ky-size-3.5" />
      Send
    </Button>
  );
}
