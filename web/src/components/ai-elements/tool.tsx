import { type ComponentProps, type ReactNode } from "react";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Badge } from "@/components/ui/badge";
import {
  CheckCircle2Icon,
  ChevronDownIcon,
  CircleIcon,
  WrenchIcon,
  XCircleIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";

/** The lifecycle state of a tool call, matching AI SDK's tool UI part states. */
export type ToolState =
  | "input-streaming"
  | "input-available"
  | "output-available"
  | "output-error";

/** A tool call/result card. Mirrors AI Elements `Tool` family. */
export function Tool({
  className,
  defaultOpen = false,
  ...props
}: ComponentProps<typeof Collapsible> & { defaultOpen?: boolean }) {
  return (
    <Collapsible
      defaultOpen={defaultOpen}
      className={cn("rounded-md border bg-muted/30", className)}
      {...props}
    />
  );
}

const STATE_META: Record<ToolState, { label: string; icon: ReactNode }> = {
  "input-streaming": {
    label: "Preparing",
    icon: <CircleIcon className="size-3 animate-pulse" />,
  },
  "input-available": {
    label: "Running",
    icon: <CircleIcon className="size-3 animate-pulse" />,
  },
  "output-available": {
    label: "Done",
    icon: <CheckCircle2Icon className="size-3" />,
  },
  "output-error": {
    label: "Error",
    icon: <XCircleIcon className="size-3" />,
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
        "group flex w-full items-center gap-2 px-2.5 py-1.5 text-xs",
        className,
      )}
      {...props}
    >
      <WrenchIcon className="size-3.5 text-muted-foreground" />
      <span className="font-mono font-medium text-foreground">{type}</span>
      <Badge
        variant="secondary"
        className={cn(
          "ml-1 gap-1 px-1.5 py-0 text-[10px] font-normal",
          state === "output-error" && "bg-destructive/10 text-destructive",
        )}
      >
        {meta.icon}
        {meta.label}
      </Badge>
      <ChevronDownIcon className="ml-auto size-3.5 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
    </CollapsibleTrigger>
  );
}

export function ToolContent({
  className,
  ...props
}: ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      className={cn("space-y-2 border-t px-3 py-2", className)}
      {...props}
    />
  );
}

export function ToolInput({ input }: { input: unknown }) {
  if (input == null) return null;
  return (
    <div className="space-y-1">
      <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
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
      <div className="space-y-1">
        <p className="text-[10px] font-medium uppercase tracking-wide text-destructive">
          Error
        </p>
        <pre className="overflow-x-auto rounded border border-destructive/40 bg-destructive/5 p-2 text-xs text-destructive">
          {errorText}
        </pre>
      </div>
    );
  }
  if (output == null) return null;
  return (
    <div className="space-y-1">
      <p className="text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
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
    <pre className="max-h-64 overflow-auto rounded border bg-background p-2 font-mono text-xs">
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
