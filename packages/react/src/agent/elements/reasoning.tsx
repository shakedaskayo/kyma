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
} from "../../internal/ui/collapsible";
import { BrainIcon, ChevronDownIcon } from "lucide-react";
import { cn } from "../../internal/cn";
import { Response } from "./response";

/**
 * Collapsible "thinking" panel. Auto-opens while the model is reasoning and
 * auto-collapses shortly after it finishes.
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
        className={cn("pv-not-prose", className)}
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
        "pv-group pv-flex pv-items-center pv-gap-1.5 pv-text-xs pv-text-muted-foreground pv-transition-colors hover:pv-text-foreground",
        className,
      )}
      {...props}
    >
      <BrainIcon className="pv-size-3.5" />
      {children ?? (
        <span>{isStreaming ? "Thinking…" : "Reasoning"}</span>
      )}
      <ChevronDownIcon className="pv-size-3.5 pv-transition-transform group-data-[state=open]:pv-rotate-180" />
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
        "pv-mt-2 pv-border-l-2 pv-border-muted pv-pl-3 pv-text-sm pv-text-muted-foreground",
        className,
      )}
      {...props}
    >
      <Response className="pv-text-[13px]">{children}</Response>
    </CollapsibleContent>
  );
}
