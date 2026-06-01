import { memo, type ComponentProps } from "react";
import { Streamdown } from "streamdown";
import { cn } from "@/lib/utils";

/**
 * Streaming-markdown renderer. Built on Vercel's `streamdown`, which handles
 * unterminated / mid-stream markdown gracefully (the prose stays well-formed
 * while tokens are still arriving) and ships Shiki code highlighting.
 *
 * Mirrors the AI Elements `Response` component API: pass the markdown string
 * as `children`.
 */
export type ResponseProps = ComponentProps<typeof Streamdown>;

export const Response = memo(
  ({ className, ...props }: ResponseProps) => (
    <Streamdown
      className={cn(
        "size-full space-y-3 text-[15px] leading-relaxed [&>*:first-child]:mt-0 [&>*:last-child]:mb-0",
        className,
      )}
      {...props}
    />
  ),
  (prev, next) => prev.children === next.children,
);

Response.displayName = "Response";
