/** BreadcrumbTrail — visited nodes, click to fly back (jumpTrail). */
import { ChevronRight } from "lucide-react";
import type { GraphNode } from "@kyma-ai/client";
import { useGraphStore } from "./graph-store";

export function BreadcrumbTrail({ nodesByCompositeId }: { nodesByCompositeId: Map<string, GraphNode> }) {
  const trail = useGraphStore((s) => s.trail);
  const jumpTrail = useGraphStore((s) => s.jumpTrail);
  if (trail.length === 0) return null;
  const visible = trail.slice(-5);
  const offset = trail.length - visible.length;
  return (
    <div className="ky-absolute ky-left-4 ky-top-6 ky-z-20 ky-flex ky-max-w-[60%] ky-items-center ky-gap-0.5 ky-rounded-full ky-glass ky-border ky-border-border ky-px-2 ky-py-1">
      {offset > 0 && <span className="ky-text-2xs ky-text-muted-foreground">…</span>}
      {visible.map((id, i) => {
        const node = nodesByCompositeId.get(id);
        const name = node ? (node.properties?.name as string) || node.id : id.split("::").pop();
        const last = i === visible.length - 1;
        return (
          <span key={`${id}-${i}`} className="ky-flex ky-items-center ky-gap-0.5">
            {(i > 0 || offset > 0) && <ChevronRight className="ky-h-3 ky-w-3 ky-text-muted-foreground" />}
            <button
              type="button"
              onClick={() => jumpTrail(offset + i)}
              className={`ky-max-w-32 ky-truncate ky-text-xs ${last ? "ky-text-foreground" : "ky-text-muted-foreground hover:ky-text-foreground"}`}
            >
              {name}
            </button>
          </span>
        );
      })}
    </div>
  );
}
