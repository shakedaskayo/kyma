import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ComponentProps,
} from "react";
import { ArrowDown } from "lucide-react";
import { Button } from "../../internal/ui/button";
import { cn } from "../../internal/cn";

/**
 * Auto-scrolling chat viewport. Sticks to the bottom as content streams in,
 * but releases once the user scrolls up and shows a "scroll to bottom" affordance.
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
    <div
      className={cn("ky-relative ky-flex-1 ky-overflow-hidden", className)}
      {...props}
    >
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="ky-h-full ky-overflow-y-auto"
      >
        {children}
      </div>
      {!atBottom && (
        <Button
          type="button"
          size="icon"
          variant="outline"
          onClick={() => scrollToBottom()}
          className="ky-absolute ky-bottom-4 ky-left-1/2 ky-size-8 -ky-translate-x-1/2 ky-rounded-full ky-shadow-md"
          aria-label="Scroll to bottom"
        >
          <ArrowDown className="ky-size-4" />
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
      className={cn(
        "ky-mx-auto ky-max-w-3xl ky-space-y-4 ky-px-4 ky-py-4",
        className,
      )}
      {...props}
    />
  );
}
