import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ComponentProps,
} from "react";
import { ArrowDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/**
 * Auto-scrolling chat viewport. Sticks to the bottom as content streams in,
 * but releases once the user scrolls up (so they can read history without
 * being yanked back down) and shows a "scroll to bottom" affordance.
 *
 * Mirrors the AI Elements `Conversation` / `ConversationContent` API; the
 * scroll-to-bottom button is built in.
 */
export function Conversation({
  className,
  children,
  ...props
}: ComponentProps<"div">) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [atBottom, setAtBottom] = useState(true);
  const atBottomRef = useRef(true);
  atBottomRef.current = atBottom;

  const onScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    setAtBottom(distance < 80);
  }, []);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "smooth") => {
    const el = scrollRef.current;
    if (el) el.scrollTo({ top: el.scrollHeight, behavior });
  }, []);

  // Keep pinned to the bottom while content grows, unless the user scrolled up.
  useEffect(() => {
    const el = scrollRef.current;
    const content = el?.firstElementChild;
    if (!el || !content) return;
    const ro = new ResizeObserver(() => {
      if (atBottomRef.current) el.scrollTop = el.scrollHeight;
    });
    ro.observe(content);
    return () => ro.disconnect();
  }, []);

  return (
    <div className={cn("relative flex-1 overflow-hidden", className)} {...props}>
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="h-full overflow-y-auto"
      >
        {children}
      </div>
      {!atBottom && (
        <Button
          type="button"
          size="icon"
          variant="outline"
          onClick={() => scrollToBottom()}
          className="absolute bottom-4 left-1/2 size-8 -translate-x-1/2 rounded-full shadow-md"
          aria-label="Scroll to bottom"
        >
          <ArrowDown className="size-4" />
        </Button>
      )}
    </div>
  );
}

export function ConversationContent({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      className={cn("mx-auto max-w-3xl space-y-4 px-4 py-4", className)}
      {...props}
    />
  );
}
