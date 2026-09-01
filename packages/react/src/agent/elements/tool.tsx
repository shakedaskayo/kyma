import { type ComponentProps, type ReactNode } from "react";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "../../internal/ui/collapsible";
import { Badge } from "../../internal/ui/badge";
import {
  CheckCircle2Icon,
  ChevronDownIcon,
  CircleIcon,
  WrenchIcon,
  XCircleIcon,
} from "lucide-react";
import { cn } from "../../internal/cn";

/** The lifecycle state of a tool call, matching AI SDK's tool UI part states. */
export type ToolState =
  | "input-streaming"
  | "input-available"
  | "output-available"
  | "output-error";

/** A tool call/result card. */
export function Tool({
  className,
  defaultOpen = false,
  ...props
}: ComponentProps<typeof Collapsible> & { defaultOpen?: boolean }) {
  return (
    <Collapsible
      defaultOpen={defaultOpen}
      className={cn("pv-rounded-md pv-border pv-bg-muted/30", className)}
      {...props}
    />
  );
}

const STATE_META: Record<ToolState, { label: string; icon: ReactNode }> = {
  "input-streaming": {
    label: "Preparing",
    icon: <CircleIcon className="pv-size-3 pv-animate-pulse" />,
  },
  "input-available": {
    label: "Running",
    icon: <CircleIcon className="pv-size-3 pv-animate-pulse" />,
  },
  "output-available": {
    label: "Done",
    icon: <CheckCircle2Icon className="pv-size-3" />,
  },
  "output-error": {
    label: "Error",
    icon: <XCircleIcon className="pv-size-3" />,
  },
};

export function ToolHeader({
  className,
  type,
  state,
  ...props
}: Omit<ComponentProps<typeof CollapsibleTrigger>, "type"> & {
  type: string;
  state: ToolState;
}) {
  const meta = STATE_META[state];
  return (
    <CollapsibleTrigger
      className={cn(
        "pv-group pv-flex pv-w-full pv-items-center pv-gap-2 pv-px-2.5 pv-py-1.5 pv-text-xs",
        className,
      )}
      {...props}
    >
      <WrenchIcon className="pv-size-3.5 pv-text-muted-foreground" />
      <span className="pv-font-mono pv-font-medium pv-text-foreground">
        {type}
      </span>
      <Badge
        variant="secondary"
        className={cn(
          "pv-ml-1 pv-gap-1 pv-px-1.5 pv-py-0 pv-text-[10px] pv-font-normal",
          state === "output-error" &&
            "pv-bg-destructive/10 pv-text-destructive",
        )}
      >
        {meta.icon}
        {meta.label}
      </Badge>
      <ChevronDownIcon className="pv-ml-auto pv-size-3.5 pv-text-muted-foreground pv-transition-transform group-data-[state=open]:pv-rotate-180" />
    </CollapsibleTrigger>
  );
}

export function ToolContent({
  className,
  ...props
}: ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      className={cn("pv-space-y-2 pv-border-t pv-px-3 pv-py-2", className)}
      {...props}
    />
  );
}

export function ToolInput({ input }: { input: unknown }) {
  if (input == null) return null;
  return (
    <div className="pv-space-y-1">
      <p className="pv-text-[10px] pv-font-medium pv-uppercase pv-tracking-wide pv-text-muted-foreground">
        Input
      </p>
      <JsonBlock value={input} />
    </div>
  );
}

export function ToolOutput({
  output,
  errorText,
}: {
  output?: unknown;
  errorText?: string;
}) {
  if (errorText) {
    return (
      <div className="pv-space-y-1">
        <p className="pv-text-[10px] pv-font-medium pv-uppercase pv-tracking-wide pv-text-destructive">
          Error
        </p>
        <pre className="pv-overflow-x-auto pv-rounded pv-border pv-border-destructive/40 pv-bg-destructive/5 pv-p-2 pv-text-xs pv-text-destructive">
          {errorText}
        </pre>
      </div>
    );
  }
  if (output == null) return null;
  return (
    <div className="pv-space-y-1">
      <p className="pv-text-[10px] pv-font-medium pv-uppercase pv-tracking-wide pv-text-muted-foreground">
        Output
      </p>
      <JsonBlock value={output} />
    </div>
  );
}

function JsonBlock({ value }: { value: unknown }) {
  const text =
    typeof value === "string" ? value : safeStringify(value);
  return (
    <pre className="pv-max-h-64 pv-overflow-auto pv-rounded pv-border pv-bg-background pv-p-2 pv-font-mono pv-text-xs">
      {text}
    </pre>
  );
}

function safeStringify(value: unknown): string {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
