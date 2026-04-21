import { Trash2, Settings } from "lucide-react";
import type { TimeRange } from "@/features/tabs/workspace-store";
import type { DashboardPanel } from "@/sdk/dashboards";
import { ChartPanelViz } from "./panels/ChartPanelViz";
import { TablePanelViz } from "./panels/TablePanelViz";
import { MarkdownPanelViz } from "./panels/MarkdownPanelViz";
import { StatPanelViz } from "./panels/StatPanelViz";

interface Props {
  panel: DashboardPanel;
  timeRange: TimeRange;
  editMode: boolean;
  onDelete: (id: string) => void;
  onEdit: (panel: DashboardPanel) => void;
}

export function PanelCard({ panel, timeRange, editMode, onDelete, onEdit }: Props) {
  return (
    <div className="flex h-full flex-col overflow-hidden rounded-md border bg-background shadow-sm">
      {/* Title bar */}
      <div
        className={`flex items-center justify-between border-b bg-muted/30 px-3 py-1.5 text-xs font-medium select-none ${
          editMode ? "cursor-grab active:cursor-grabbing" : ""
        }`}
      >
        <span className="truncate text-foreground">{panel.title}</span>
        {editMode && (
          <div className="flex items-center gap-1 shrink-0 ml-2">
            <button
              className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
              title="Edit panel"
              onClick={(e) => {
                e.stopPropagation();
                onEdit(panel);
              }}
            >
              <Settings className="h-3.5 w-3.5" />
            </button>
            <button
              className="rounded p-0.5 text-muted-foreground hover:bg-destructive/10 hover:text-destructive transition-colors"
              title="Delete panel"
              onClick={(e) => {
                e.stopPropagation();
                onDelete(panel.id);
              }}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          </div>
        )}
      </div>

      {/* Panel body */}
      <div className="min-h-0 flex-1 overflow-hidden">
        <PanelBody panel={panel} timeRange={timeRange} />
      </div>
    </div>
  );
}

function PanelBody({ panel, timeRange }: { panel: DashboardPanel; timeRange: TimeRange }) {
  switch (panel.panel_type) {
    case "chart":
      return <ChartPanelViz panel={panel} timeRange={timeRange} />;
    case "table":
      return <TablePanelViz panel={panel} timeRange={timeRange} />;
    case "markdown":
      return <MarkdownPanelViz panel={panel} />;
    case "stat":
      return <StatPanelViz panel={panel} timeRange={timeRange} />;
    default:
      return (
        <div className="flex h-full items-center justify-center text-xs text-muted-foreground">
          Unknown panel type.
        </div>
      );
  }
}
