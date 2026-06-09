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
      className={cn("ky-rounded-md ky-border ky-bg-muted/30", className)}
      {...props}
    />
  );
}

const STATE_META: Record<ToolState, { label: string; icon: ReactNode }> = {
  "input-streaming": {
    label: "Preparing",
    icon: <CircleIcon className="ky-size-3 ky-animate-pulse" />,
  },
  "input-available": {
    label: "Running",
    icon: <CircleIcon className="ky-size-3 ky-animate-pulse" />,
  },
  "output-available": {
    label: "Done",
    icon: <CheckCircle2Icon className="ky-size-3" />,
  },
  "output-error": {
    label: "Error",
    icon: <XCircleIcon className="ky-size-3" />,
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
        "ky-group ky-flex ky-w-full ky-items-center ky-gap-2 ky-px-2.5 ky-py-1.5 ky-text-xs",
        className,
      )}
      {...props}
    >
      <WrenchIcon className="ky-size-3.5 ky-text-muted-foreground" />
      <span className="ky-font-mono ky-font-medium ky-text-foreground">
        {type}
      </span>
      <Badge
        variant="secondary"
        className={cn(
          "ky-ml-1 ky-gap-1 ky-px-1.5 ky-py-0 ky-text-[10px] ky-font-normal",
          state === "output-error" &&
            "ky-bg-destructive/10 ky-text-destructive",
        )}
      >
        {meta.icon}
        {meta.label}
      </Badge>
      <ChevronDownIcon className="ky-ml-auto ky-size-3.5 ky-text-muted-foreground ky-transition-transform group-data-[state=open]:ky-rotate-180" />
    </CollapsibleTrigger>
  );
}

export function ToolContent({
  className,
  ...props
}: ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      className={cn("ky-space-y-2 ky-border-t ky-px-3 ky-py-2", className)}
      {...props}
    />
  );
}

export function ToolInput({ input }: { input: unknown }) {
  if (input == null) return null;
  return (
    <div className="ky-space-y-1">
      <p className="ky-text-[10px] ky-font-medium ky-uppercase ky-tracking-wide ky-text-muted-foreground">
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
      <div className="ky-space-y-1">
        <p className="ky-text-[10px] ky-font-medium ky-uppercase ky-tracking-wide ky-text-destructive">
          Error
        </p>
        <pre className="ky-overflow-x-auto ky-rounded ky-border ky-border-destructive/40 ky-bg-destructive/5 ky-p-2 ky-text-xs ky-text-destructive">
          {errorText}
        </pre>
      </div>
    );
  }
  if (output == null) return null;
  return (
    <div className="ky-space-y-1">
      <p className="ky-text-[10px] ky-font-medium ky-uppercase ky-tracking-wide ky-text-muted-foreground">
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
    <pre className="ky-max-h-64 ky-overflow-auto ky-rounded ky-border ky-bg-background ky-p-2 ky-font-mono ky-text-xs">
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
