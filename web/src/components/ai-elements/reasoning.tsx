import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ComponentProps,
} from "react";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { BrainIcon, ChevronDownIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { Response } from "./response";

/**
 * Collapsible "thinking" panel. Auto-opens while the model is reasoning and
 * auto-collapses shortly after it finishes, so the trace is visible live but
 * tucked away once the answer arrives. Mirrors AI Elements `Reasoning`.
 */
type ReasoningContextValue = { isStreaming: boolean };
const ReasoningContext = createContext<ReasoningContextValue>({
  isStreaming: false,
});

export function Reasoning({
  className,
  isStreaming = false,
  open: openProp,
  defaultOpen = false,
  onOpenChange,
  children,
  ...props
}: ComponentProps<typeof Collapsible> & { isStreaming?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const wasStreaming = useRef(isStreaming);

  // Open as soon as reasoning starts; collapse a beat after it ends.
  useEffect(() => {
    if (isStreaming && !wasStreaming.current) {
      setOpen(true);
    } else if (!isStreaming && wasStreaming.current) {
      const t = setTimeout(() => setOpen(false), 800);
      wasStreaming.current = isStreaming;
      return () => clearTimeout(t);
    }
    wasStreaming.current = isStreaming;
  }, [isStreaming]);

  const handleOpenChange = (next: boolean) => {
    setOpen(next);
    onOpenChange?.(next);
  };

  return (
    <ReasoningContext.Provider value={{ isStreaming }}>
      <Collapsible
        className={cn("not-prose", className)}
        open={openProp ?? open}
        onOpenChange={handleOpenChange}
        {...props}
      >
        {children}
      </Collapsible>
    </ReasoningContext.Provider>
  );
}

export function ReasoningTrigger({
  className,
  children,
  ...props
}: ComponentProps<typeof CollapsibleTrigger>) {
  const { isStreaming } = useContext(ReasoningContext);
  return (
    <CollapsibleTrigger
      className={cn(
        "group flex items-center gap-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground",
        className,
      )}
      {...props}
    >
      <BrainIcon className="size-3.5" />
      {children ?? <span>{isStreaming ? "Thinking…" : "Reasoning"}</span>}
      <ChevronDownIcon className="size-3.5 transition-transform group-data-[state=open]:rotate-180" />
    </CollapsibleTrigger>
  );
}

export function ReasoningContent({
  className,
  children,
  ...props
}: Omit<ComponentProps<typeof CollapsibleContent>, "children"> & {
  children: string;
}) {
  return (
    <CollapsibleContent
      className={cn(
        "mt-2 border-l-2 border-muted pl-3 text-sm text-muted-foreground",
        className,
      )}
      {...props}
    >
      <Response className="text-[13px]">{children}</Response>
    </CollapsibleContent>
  );
}
