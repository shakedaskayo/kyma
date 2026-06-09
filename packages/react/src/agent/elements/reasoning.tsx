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
        className={cn("ky-not-prose", className)}
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
        "ky-group ky-flex ky-items-center ky-gap-1.5 ky-text-xs ky-text-muted-foreground ky-transition-colors hover:ky-text-foreground",
        className,
      )}
      {...props}
    >
      <BrainIcon className="ky-size-3.5" />
      {children ?? (
        <span>{isStreaming ? "Thinking…" : "Reasoning"}</span>
      )}
      <ChevronDownIcon className="ky-size-3.5 ky-transition-transform group-data-[state=open]:ky-rotate-180" />
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
        "ky-mt-2 ky-border-l-2 ky-border-muted ky-pl-3 ky-text-sm ky-text-muted-foreground",
        className,
      )}
      {...props}
    >
      <Response className="ky-text-[13px]">{children}</Response>
    </CollapsibleContent>
  );
}
