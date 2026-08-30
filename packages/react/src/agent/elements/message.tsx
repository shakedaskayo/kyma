import { type ComponentProps } from "react";
import { cn } from "../../internal/cn";

export type MessageRole = "user" | "assistant" | "system";

export function Message({
  className,
  from,
  ...props
}: ComponentProps<"div"> & { from: MessageRole }) {
  return (
    <div
      className={cn(
        "pv-group pv-flex pv-w-full",
        from === "user" ? "pv-justify-end" : "pv-justify-start",
        className,
      )}
      data-role={from}
      {...props}
    />
  );
}

export function MessageContent({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "pv-flex pv-min-w-0 pv-flex-col pv-gap-2 pv-overflow-hidden pv-rounded-lg pv-text-sm",
        // User: compact accent bubble.
        "group-data-[role=user]:pv-max-w-[80%] group-data-[role=user]:pv-bg-primary group-data-[role=user]:pv-px-3 group-data-[role=user]:pv-py-2 group-data-[role=user]:pv-text-primary-foreground group-data-[role=user]:pv-shadow-sm",
        // Assistant: full-width card.
        "group-data-[role=assistant]:pv-w-full group-data-[role=assistant]:pv-border group-data-[role=assistant]:pv-bg-card group-data-[role=assistant]:pv-px-4 group-data-[role=assistant]:pv-py-3 group-data-[role=assistant]:pv-text-card-foreground group-data-[role=assistant]:pv-shadow-sm",
        className,
      )}
      {...props}
    />
  );
}
