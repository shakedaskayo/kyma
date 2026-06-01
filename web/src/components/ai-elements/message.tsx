import { type ComponentProps } from "react";
import { cn } from "@/lib/utils";

export type MessageRole = "user" | "assistant" | "system";

/**
 * One message row. `from` drives alignment + bubble styling via a `group`
 * data attribute that `MessageContent` reads. Mirrors AI Elements `Message`.
 */
export function Message({
  className,
  from,
  ...props
}: ComponentProps<"div"> & { from: MessageRole }) {
  return (
    <div
      className={cn(
        "group flex w-full",
        from === "user" ? "justify-end" : "justify-start",
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
        "flex min-w-0 flex-col gap-2 overflow-hidden rounded-lg text-sm",
        // User: compact accent bubble.
        "group-data-[role=user]:max-w-[80%] group-data-[role=user]:bg-primary group-data-[role=user]:px-3 group-data-[role=user]:py-2 group-data-[role=user]:text-primary-foreground group-data-[role=user]:shadow-sm",
        // Assistant: full-width card.
        "group-data-[role=assistant]:w-full group-data-[role=assistant]:border group-data-[role=assistant]:bg-card group-data-[role=assistant]:px-4 group-data-[role=assistant]:py-3 group-data-[role=assistant]:text-card-foreground group-data-[role=assistant]:shadow-sm",
        className,
      )}
      {...props}
    />
  );
}
