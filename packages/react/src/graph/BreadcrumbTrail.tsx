/** BreadcrumbTrail — visited nodes, click to fly back (jumpTrail). */
import { ChevronRight } from "lucide-react";
import type { GraphNode } from "@pensieve-ai/client";
import { useGraphStore } from "./graph-store";

export function BreadcrumbTrail({ nodesByCompositeId }: { nodesByCompositeId: Map<string, GraphNode> }) {
  const trail = useGraphStore((s) => s.trail);
  const jumpTrail = useGraphStore((s) => s.jumpTrail);
  if (trail.length === 0) return null;
  const visible = trail.slice(-5);
  const offset = trail.length - visible.length;
  return (
    <div className="pv-absolute pv-left-4 pv-top-6 pv-z-20 pv-flex pv-max-w-[60%] pv-items-center pv-gap-0.5 pv-rounded-full pv-glass pv-border pv-border-border pv-px-2 pv-py-1">
      {offset > 0 && <span className="pv-text-2xs pv-text-muted-foreground">…</span>}
      {visible.map((id, i) => {
        const node = nodesByCompositeId.get(id);
        const name = node ? (node.properties?.name as string) || node.id : id.split("::").pop();
        const last = i === visible.length - 1;
        return (
          <span key={`${id}-${i}`} className="pv-flex pv-items-center pv-gap-0.5">
            {(i > 0 || offset > 0) && <ChevronRight className="pv-h-3 pv-w-3 pv-text-muted-foreground" />}
            <button
              type="button"
              onClick={() => jumpTrail(offset + i)}
              className={`pv-max-w-32 pv-truncate pv-text-xs ${last ? "pv-text-foreground" : "pv-text-muted-foreground hover:pv-text-foreground"}`}
            >
              {name}
            </button>
          </span>
        );
      })}
    </div>
  );
}
