/**
 * PanelCard — renders a single dashboard panel in a card frame.
 *
 * Shows: title bar (with edit/delete controls in editable mode), panel body.
 * The title bar doubles as a drag handle when the grid is draggable (the
 * parent passes the handle class via draggableHandle on GridLayout).
 */

import { Trash2, Settings } from "lucide-react";
import type { DashboardPanel } from "@pensieve-ai/client";
import type { TimeRange } from "../query/time-range/time-range-types";
import { ChartPanelViz } from "./panels/ChartPanelViz";
import { TablePanelViz } from "./panels/TablePanelViz";
import { MarkdownPanelViz } from "./panels/MarkdownPanelViz";
import { StatPanelViz } from "./panels/StatPanelViz";

interface Props {
  panel: DashboardPanel;
  timeRange: TimeRange;
  editable: boolean;
  onDelete: (id: string) => void;
  onEdit: (panel: DashboardPanel) => void;
  onPanelClick?: (panelId: string) => void;
}

export function PanelCard({ panel, timeRange, editable, onDelete, onEdit, onPanelClick }: Props) {
  return (
    <div
      className="ky-flex ky-h-full ky-flex-col ky-overflow-hidden ky-rounded-md ky-border ky-bg-background ky-shadow-sm"
      onClick={onPanelClick ? () => onPanelClick(panel.id) : undefined}
    >
      {/* Title bar — also serves as drag handle when editable */}
      <div
        className={`ky-flex ky-items-center ky-justify-between ky-border-b ky-bg-muted/30 ky-px-3 ky-py-1.5 ky-text-xs ky-font-medium ky-select-none pensieve-panel-drag-handle ${
          editable ? "ky-cursor-grab active:ky-cursor-grabbing" : ""
        }`}
      >
        <span className="ky-truncate ky-text-foreground">{panel.title}</span>
        {editable && (
          <div className="ky-flex ky-items-center ky-gap-1 ky-shrink-0 ky-ml-2">
            <button
              className="ky-rounded ky-p-0.5 ky-text-muted-foreground hover:ky-bg-accent hover:ky-text-foreground ky-transition-colors"
              title="Edit panel"
              onClick={(e) => {
                e.stopPropagation();
                onEdit(panel);
              }}
            >
              <Settings className="ky-h-3.5 ky-w-3.5" />
            </button>
            <button
              className="ky-rounded ky-p-0.5 ky-text-muted-foreground hover:ky-bg-destructive/10 hover:ky-text-destructive ky-transition-colors"
              title="Delete panel"
              onClick={(e) => {
                e.stopPropagation();
                onDelete(panel.id);
              }}
            >
              <Trash2 className="ky-h-3.5 ky-w-3.5" />
            </button>
          </div>
        )}
      </div>

      {/* Panel body */}
      <div className="ky-min-h-0 ky-flex-1 ky-overflow-hidden">
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
        <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-text-xs ky-text-muted-foreground">
          Unknown panel type.
        </div>
      );
  }
}
