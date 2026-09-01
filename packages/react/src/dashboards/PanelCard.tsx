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
      className="pv-flex pv-h-full pv-flex-col pv-overflow-hidden pv-rounded-md pv-border pv-bg-background pv-shadow-sm"
      onClick={onPanelClick ? () => onPanelClick(panel.id) : undefined}
    >
      {/* Title bar — also serves as drag handle when editable */}
      <div
        className={`pv-flex pv-items-center pv-justify-between pv-border-b pv-bg-muted/30 pv-px-3 pv-py-1.5 pv-text-xs pv-font-medium pv-select-none pensieve-panel-drag-handle ${
          editable ? "pv-cursor-grab active:pv-cursor-grabbing" : ""
        }`}
      >
        <span className="pv-truncate pv-text-foreground">{panel.title}</span>
        {editable && (
          <div className="pv-flex pv-items-center pv-gap-1 pv-shrink-0 pv-ml-2">
            <button
              className="pv-rounded pv-p-0.5 pv-text-muted-foreground hover:pv-bg-accent hover:pv-text-foreground pv-transition-colors"
              title="Edit panel"
              onClick={(e) => {
                e.stopPropagation();
                onEdit(panel);
              }}
            >
              <Settings className="pv-h-3.5 pv-w-3.5" />
            </button>
            <button
              className="pv-rounded pv-p-0.5 pv-text-muted-foreground hover:pv-bg-destructive/10 hover:pv-text-destructive pv-transition-colors"
              title="Delete panel"
              onClick={(e) => {
                e.stopPropagation();
                onDelete(panel.id);
              }}
            >
              <Trash2 className="pv-h-3.5 pv-w-3.5" />
            </button>
          </div>
        )}
      </div>

      {/* Panel body */}
      <div className="pv-min-h-0 pv-flex-1 pv-overflow-hidden">
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
        <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-text-xs pv-text-muted-foreground">
          Unknown panel type.
        </div>
      );
  }
}
