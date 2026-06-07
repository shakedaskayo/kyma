import { Moon, Workflow } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import type { RunKind } from "@/sdk/dreaming";

/** Run kind chip: violet + Moon for dreaming, slate + Workflow for consolidation. */
export function RunKindBadge({ kind, className }: { kind: RunKind; className?: string }) {
  if (kind === "dreaming") {
    return (
      <Badge tone="violet" className={className}>
        <Moon className="h-3 w-3" /> Dreaming
      </Badge>
    );
  }
  return (
    <Badge tone="slate" className={className}>
      <Workflow className="h-3 w-3" /> Consolidation
    </Badge>
  );
}
