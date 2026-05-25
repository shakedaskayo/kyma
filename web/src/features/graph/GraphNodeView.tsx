import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";

function GraphNodeViewInner({ data }: NodeProps) {
  const d = data as { label: string; labels: string[]; color: string; isSelected: boolean; isDimmed: boolean };
  const color = d.color;
  const size = d.isSelected ? 52 : 44;
  return (
    <div
      style={{ opacity: d.isDimmed ? 0.15 : 1, transition: "opacity 0.2s" }}
      className="flex flex-col items-center"
    >
      <Handle type="target" position={Position.Top} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Bottom} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="target" position={Position.Left} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Right} style={{ opacity: 0, width: 1, height: 1 }} />
      <div
        style={{
          width: size,
          height: size,
          borderRadius: "9999px",
          background: `radial-gradient(circle at 40% 35%, ${color}40, ${color}15)`,
          border: `1.5px solid ${color}`,
          boxShadow: d.isSelected ? `0 0 20px ${color}55` : "none",
          transition: "all 0.15s",
        }}
      />
      <div
        className={`mt-1 max-w-[120px] truncate text-[10px] ${d.isSelected ? "font-semibold text-foreground" : "text-muted-foreground"}`}
        title={d.label}
      >
        {d.label}
      </div>
    </div>
  );
}

export const GraphNodeView = memo(GraphNodeViewInner);
