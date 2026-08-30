import { memo, type ComponentProps } from "react";
import { Streamdown } from "streamdown";
import { cn } from "../../internal/cn";

export type ResponseProps = ComponentProps<typeof Streamdown>;

/**
 * Streaming-markdown renderer using streamdown.
 * Handles unterminated / mid-stream markdown gracefully while tokens arrive.
 */
export const Response = memo(
  ({ className, ...props }: ResponseProps) => (
    <Streamdown
      className={cn(
        "pv-size-full pv-space-y-3 pv-text-[15px] pv-leading-relaxed [&>*:first-child]:pv-mt-0 [&>*:last-child]:pv-mb-0",
        className,
      )}
      {...props}
    />
  ),
  (prev, next) => prev.children === next.children,
);

Response.displayName = "Response";
