import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import {
  GitBranch, FileCode2, FunctionSquare, Box, Braces, FolderClosed,
  GitPullRequest, CircleDot, User as UserIcon, Table2, Database, Network,
} from "lucide-react";

/** Type icon shown inside the node disc — connector/type-specific visual cue. */
function iconFor(label: string): React.ComponentType<{ className?: string; style?: React.CSSProperties }> {
  const map: Record<string, React.ComponentType<{ className?: string; style?: React.CSSProperties }>> = {
    Repository: GitBranch, Branch: GitBranch, PullRequest: GitPullRequest, Issue: CircleDot,
    User: UserIcon, Directory: FolderClosed, CodeFile: FileCode2, CodeFunction: FunctionSquare,
    CodeClass: Braces, Module: Box, Table: Table2, Service: Box, Database,
  };
  return map[label] ?? Network;
}

function GraphNodeViewInner({ data }: NodeProps) {
  const d = data as {
    label: string;
    labels: string[];
    color: string;
    isSelected: boolean;
    isDimmed: boolean;
  };
  const color = d.color;
  const primary = d.labels[0] ?? "";
  const size = d.isSelected ? 50 : 40;
  const Icon = iconFor(primary);

  return (
    <div
      style={{ opacity: d.isDimmed ? 0.18 : 1, transition: "opacity 0.2s" }}
      className="group flex flex-col items-center"
    >
      <Handle type="target" position={Position.Top} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Bottom} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="target" position={Position.Left} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Right} style={{ opacity: 0, width: 1, height: 1 }} />

      {/* Clean disc: white fill, colored ring + icon. Reads well on a light canvas. */}
      <div
        className="flex items-center justify-center rounded-full bg-white"
        style={{
          width: size,
          height: size,
          border: `${d.isSelected ? 3 : 2}px solid ${color}`,
          boxShadow: d.isSelected
            ? `0 0 0 4px ${color}22, 0 6px 16px rgba(15,23,42,0.16)`
            : `0 2px 6px rgba(15,23,42,0.10)`,
          transition: "all 0.15s",
        }}
      >
        <Icon style={{ width: size * 0.46, height: size * 0.46, color }} />
      </div>

      {/* Name below */}
      <div
        className={`mt-1.5 max-w-[128px] truncate text-center text-[10.5px] leading-tight ${
          d.isSelected ? "font-semibold text-slate-900" : "font-medium text-slate-600 group-hover:text-slate-900"
        }`}
        title={d.label}
      >
        {d.label}
      </div>
    </div>
  );
}

export const GraphNodeView = memo(GraphNodeViewInner);
