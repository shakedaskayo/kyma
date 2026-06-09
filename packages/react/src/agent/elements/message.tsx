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
        "ky-group ky-flex ky-w-full",
        from === "user" ? "ky-justify-end" : "ky-justify-start",
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
        "ky-flex ky-min-w-0 ky-flex-col ky-gap-2 ky-overflow-hidden ky-rounded-lg ky-text-sm",
        // User: compact accent bubble.
        "group-data-[role=user]:ky-max-w-[80%] group-data-[role=user]:ky-bg-primary group-data-[role=user]:ky-px-3 group-data-[role=user]:ky-py-2 group-data-[role=user]:ky-text-primary-foreground group-data-[role=user]:ky-shadow-sm",
        // Assistant: full-width card.
        "group-data-[role=assistant]:ky-w-full group-data-[role=assistant]:ky-border group-data-[role=assistant]:ky-bg-card group-data-[role=assistant]:ky-px-4 group-data-[role=assistant]:ky-py-3 group-data-[role=assistant]:ky-text-card-foreground group-data-[role=assistant]:ky-shadow-sm",
        className,
      )}
      {...props}
    />
  );
}
