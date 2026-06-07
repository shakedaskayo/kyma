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
        "ky-size-full ky-space-y-3 ky-text-[15px] ky-leading-relaxed [&>*:first-child]:ky-mt-0 [&>*:last-child]:ky-mb-0",
        className,
      )}
      {...props}
    />
  ),
  (prev, next) => prev.children === next.children,
);

Response.displayName = "Response";
