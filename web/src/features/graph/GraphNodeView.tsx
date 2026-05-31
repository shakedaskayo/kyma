import { memo, useContext, useMemo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { useGraphStore } from "./graph-store";
import { CanvasContext } from "./canvas-context";
import { useTheme } from "@/lib/theme";

/**
 * Static, structural data the canvas pre-computes for each node. Decoupled
 * from hover/selection state — those live in the graph store and are
 * subscribed-to per-node so a hover doesn't ripple a full canvas rebuild.
 */
interface NodeData {
  label: string;
  labels: string[];
  color: string;
}

function GraphNodeViewInner({ id, data }: NodeProps) {
  const d = data as unknown as NodeData;
  const ctx = useContext(CanvasContext);
  const myNeighbors = useMemo(
    () => ctx.neighborsByCompositeId.get(id),
    [ctx.neighborsByCompositeId, id],
  );
  const isDark = useTheme((s) => s.resolved === "dark");

  // Per-node selectors that return BOOLEANS — Zustand only re-renders the
  // consumer when the selector's output changes, so a hover only flips
  // state on the (small) set of nodes whose boolean transitions (e.g.
  // enter/leave the focused node's neighbourhood). This is the difference
  // between re-rendering ~50 nodes per hover and re-rendering all of them.
  const isSelected = useGraphStore((s) => s.selectedNodeId === id);
  const isSearchMatch = useGraphStore((s) => {
    const q = s.searchQuery.trim().toLowerCase();
    if (!q) return false;
    return d.label.toLowerCase().includes(q) || id.toLowerCase().includes(q);
  });
  const isDimmed = useGraphStore((s) => {
    if (s.labelFilter !== null) return !d.labels.includes(s.labelFilter);
    if (s.relTypeFilter !== null) {
      return !ctx.nodesByRelType.get(s.relTypeFilter)?.has(id);
    }
    if (s.searchQuery.trim()) {
      const q = s.searchQuery.trim().toLowerCase();
      return !(d.label.toLowerCase().includes(q) || id.toLowerCase().includes(q));
    }
    const focusId = s.selectedNodeId ?? s.hoveredNodeId;
    if (!focusId) return false;
    if (focusId === id) return false;
    if (!myNeighbors) return true;
    return !myNeighbors.has(focusId);
  });

  const color = d.color;
  const baseRing = isDark ? "rgba(255,255,255,0.10)" : "rgba(15,23,42,0.10)";
  const selectedRing = isDark ? "#f1f5f9" : "#0f172a";
  const searchRing = "#f59e0b";
  const labelColor = isSelected
    ? "hsl(var(--foreground))"
    : isSearchMatch
      ? searchRing
      : "hsl(var(--muted-foreground))";
  const ringColor = isSelected
    ? selectedRing
    : isSearchMatch
      ? searchRing
      : baseRing;
  const size = isSelected ? 18 : isSearchMatch ? 16 : 12;
  const labelShadow = isDark
    ? "0 1px 2px rgba(0,0,0,0.85)"
    : "0 1px 2px rgba(255,255,255,0.9)";

  return (
    <div
      style={{
        opacity: isDimmed ? 0.18 : 1,
        transition: "opacity 160ms ease",
      }}
      className="group flex flex-col items-center"
    >
      <Handle type="target" position={Position.Top} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Bottom} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="target" position={Position.Left} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Right} style={{ opacity: 0, width: 1, height: 1 }} />

      {/* Dot — small solid disc, themed ring, glow on selection/search.
          Ring is box-shadow (not border) so the geometry stays constant on
          state flip (no layout shift). */}
      <div
        className="rounded-full"
        style={{
          width: size,
          height: size,
          background: color,
          boxShadow: isSelected
            ? `0 0 0 2.5px ${ringColor}, 0 0 18px ${color}aa`
            : isSearchMatch
              ? `0 0 0 2px ${ringColor}, 0 0 14px ${ringColor}55`
              : `0 0 0 1.5px ${ringColor}`,
          transition: "all 160ms ease",
        }}
      />

      <div
        className="pointer-events-none mt-1 max-w-[120px] truncate text-center text-[10px] leading-tight"
        style={{
          color: labelColor,
          fontWeight: isSelected ? 600 : 500,
          textShadow: labelShadow,
        }}
        title={d.label}
      >
        {d.label}
      </div>
    </div>
  );
}

export const GraphNodeView = memo(GraphNodeViewInner);
