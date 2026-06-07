/**
 * AddPanelModal — create/edit dashboard panel form (editable mode only).
 *
 * Adapted from web's AddPanelModal with:
 *   - session → useKymaClient() for catalog schema fetch
 *   - web shadcn components → internal/ui primitives (ky- prefix)
 *   - Toast dropped (no router-level notification infra in the SDK)
 *   - Preview uses SDK panel viz components
 */

import { useState } from "react";
import { ChevronDown, ChevronRight, Play } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import type { DashboardPanel } from "@kyma-ai/client";
import { useKymaClient } from "../provider/context";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "../internal/ui/dialog";
import { Button } from "../internal/ui/button";
import { Input } from "../internal/ui/input";
import { ChartPanelViz } from "./panels/ChartPanelViz";
import { TablePanelViz } from "./panels/TablePanelViz";
import { StatPanelViz } from "./panels/StatPanelViz";

// ── Types ─────────────────────────────────────────────────────────────────────

type PanelType = "chart" | "table" | "markdown" | "stat";

export type PanelDraft = Omit<DashboardPanel, "id" | "dashboard_id">;

interface Props {
  open: boolean;
  onClose: () => void;
  onSave: (draft: PanelDraft) => void;
  initial?: DashboardPanel | null;
}

const DEFAULT_DRAFT: PanelDraft = {
  title: "",
  panel_type: "chart",
  query: "",
  database_name: null,
  config: {},
  grid_x: 0,
  grid_y: 0,
  grid_w: 6,
  grid_h: 4,
  display_order: 0,
};

// ── MonacoMini — simple textarea for query editing ────────────────────────────

function MonacoMini({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <textarea
      className="ky-h-28 ky-w-full ky-rounded-md ky-border ky-border-input ky-bg-background ky-p-2 ky-font-mono ky-text-xs placeholder:ky-text-muted-foreground focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring ky-resize-none"
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder="Enter KQL query…"
      spellCheck={false}
    />
  );
}

// ── Label primitive ───────────────────────────────────────────────────────────

function Label({ htmlFor, children }: { htmlFor?: string; children: React.ReactNode }) {
  return (
    <label
      htmlFor={htmlFor}
      className="ky-text-sm ky-font-medium ky-leading-none peer-disabled:ky-cursor-not-allowed peer-disabled:ky-opacity-70"
    >
      {children}
    </label>
  );
}

// ── Component ─────────────────────────────────────────────────────────────────

export function AddPanelModal({ open, onClose, onSave, initial }: Props) {
  const client = useKymaClient();
  const endpoint = client.transport.endpoint;
  const database = client.transport.database ?? "default";

  const [draft, setDraft] = useState<PanelDraft>(() =>
    initial
      ? {
          title: initial.title,
          panel_type: initial.panel_type,
          query: initial.query ?? "",
          database_name: initial.database_name,
          config: { ...initial.config },
          grid_x: initial.grid_x,
          grid_y: initial.grid_y,
          grid_w: initial.grid_w,
          grid_h: initial.grid_h,
          display_order: initial.display_order,
        }
      : DEFAULT_DRAFT,
  );

  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [preview, setPreview] = useState<DashboardPanel | null>(null);
  const [previewKey, setPreviewKey] = useState(0);

  const { data: schema } = useQuery({
    queryKey: ["kyma", endpoint, database, "schema"],
    queryFn: () => client.catalog.fetchSchema(),
    staleTime: 5 * 60_000,
  });

  const databases = schema?.databases.map((d) => d.name) ?? [];

  const up = (patch: Partial<PanelDraft>) => setDraft((d) => ({ ...d, ...patch }));
  const upConfig = (patch: Record<string, unknown>) =>
    setDraft((d) => ({ ...d, config: { ...d.config, ...patch } }));

  const handleSubmit = () => {
    if (!draft.title.trim()) return;
    onSave(draft);
    onClose();
    setDraft(DEFAULT_DRAFT);
    setPreview(null);
  };

  const handlePreview = () => {
    const fakePanel: DashboardPanel = {
      id: "preview",
      dashboard_id: "preview",
      ...draft,
      query: draft.query || null,
    };
    setPreview(fakePanel);
    setPreviewKey((k) => k + 1);
  };

  const panelTypes: { value: PanelType; label: string }[] = [
    { value: "chart", label: "Chart" },
    { value: "table", label: "Table" },
    { value: "markdown", label: "Markdown" },
    { value: "stat", label: "Stat" },
  ];

  const isMarkdown = draft.panel_type === "markdown";
  const isChart = draft.panel_type === "chart";
  const isTable = draft.panel_type === "table";
  const isStat = draft.panel_type === "stat";

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="ky-max-w-2xl ky-max-h-[90vh] ky-overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{initial ? "Edit panel" : "Add panel"}</DialogTitle>
        </DialogHeader>

        <div className="ky-space-y-4 ky-py-1">
          {/* Title */}
          <div className="ky-space-y-1">
            <Label htmlFor="panel-title">Title *</Label>
            <Input
              id="panel-title"
              value={draft.title}
              onChange={(e) => up({ title: e.target.value })}
              placeholder="Panel title"
            />
          </div>

          {/* Type */}
          <div className="ky-space-y-1">
            <Label>Type</Label>
            <div className="ky-flex ky-gap-2">
              {panelTypes.map((pt) => (
                <button
                  key={pt.value}
                  onClick={() => up({ panel_type: pt.value })}
                  className={`ky-rounded-md ky-border ky-px-3 ky-py-1.5 ky-text-xs ky-font-medium ky-transition ${
                    draft.panel_type === pt.value
                      ? "ky-border-primary ky-bg-primary ky-text-primary-foreground"
                      : "ky-border-border ky-text-muted-foreground hover:ky-border-primary/50 hover:ky-text-foreground"
                  }`}
                >
                  {pt.label}
                </button>
              ))}
            </div>
          </div>

          {/* Database */}
          {!isMarkdown && (
            <div className="ky-space-y-1">
              <Label htmlFor="panel-db">Database</Label>
              <select
                id="panel-db"
                className="ky-w-full ky-rounded-md ky-border ky-border-input ky-bg-background ky-px-3 ky-py-2 ky-text-sm focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring"
                value={draft.database_name ?? ""}
                onChange={(e) => up({ database_name: e.target.value || null })}
              >
                <option value="">(session default)</option>
                {databases.map((db) => (
                  <option key={db} value={db}>
                    {db}
                  </option>
                ))}
              </select>
            </div>
          )}

          {/* Query */}
          {!isMarkdown && (
            <div className="ky-space-y-1">
              <Label>Query</Label>
              <MonacoMini value={draft.query ?? ""} onChange={(v) => up({ query: v })} />
            </div>
          )}

          {/* Markdown textarea */}
          {isMarkdown && (
            <div className="ky-space-y-1">
              <Label>Markdown content</Label>
              <textarea
                className="ky-h-40 ky-w-full ky-rounded-md ky-border ky-border-input ky-bg-background ky-p-2 ky-font-mono ky-text-xs focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring ky-resize-none"
                value={(draft.config.markdown as string | undefined) ?? ""}
                onChange={(e) => upConfig({ markdown: e.target.value })}
                placeholder="# My panel&#10;&#10;Write **markdown** here."
              />
            </div>
          )}

          {/* Table: max rows */}
          {isTable && (
            <div className="ky-space-y-1">
              <Label htmlFor="panel-maxrows">Max rows</Label>
              <Input
                id="panel-maxrows"
                type="number"
                min={1}
                max={1000}
                value={(draft.config.maxRows as number | undefined) ?? 50}
                onChange={(e) => upConfig({ maxRows: parseInt(e.target.value, 10) || 50 })}
              />
            </div>
          )}

          {/* Stat settings */}
          {isStat && (
            <div className="ky-space-y-3">
              <div className="ky-space-y-1">
                <Label htmlFor="stat-label">Label</Label>
                <Input
                  id="stat-label"
                  value={(draft.config.label as string | undefined) ?? ""}
                  onChange={(e) => upConfig({ label: e.target.value })}
                  placeholder="e.g. requests/sec"
                />
              </div>
              <div className="ky-space-y-1">
                <Label htmlFor="stat-valuecol">Value column (optional)</Label>
                <Input
                  id="stat-valuecol"
                  value={(draft.config.valueCol as string | undefined) ?? ""}
                  onChange={(e) => upConfig({ valueCol: e.target.value || undefined })}
                  placeholder="auto-detect"
                />
              </div>
              <div className="ky-space-y-1">
                <Label htmlFor="stat-format">Format</Label>
                <select
                  id="stat-format"
                  className="ky-w-full ky-rounded-md ky-border ky-border-input ky-bg-background ky-px-3 ky-py-2 ky-text-sm focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring"
                  value={(draft.config.format as string | undefined) ?? "number"}
                  onChange={(e) => upConfig({ format: e.target.value })}
                >
                  <option value="number">Number</option>
                  <option value="duration_ms">Duration (ms)</option>
                  <option value="percent">Percent</option>
                  <option value="bytes">Bytes</option>
                </select>
              </div>
            </div>
          )}

          {/* Chart advanced */}
          {isChart && (
            <div>
              <button
                type="button"
                className="ky-flex ky-items-center ky-gap-1 ky-text-xs ky-text-muted-foreground hover:ky-text-foreground"
                onClick={() => setAdvancedOpen((v) => !v)}
              >
                {advancedOpen ? (
                  <ChevronDown className="ky-h-3.5 ky-w-3.5" />
                ) : (
                  <ChevronRight className="ky-h-3.5 ky-w-3.5" />
                )}
                Advanced chart settings
              </button>
              {advancedOpen && (
                <div className="ky-mt-2 ky-space-y-3 ky-rounded-md ky-border ky-bg-muted/20 ky-p-3">
                  <div className="ky-space-y-1">
                    <Label htmlFor="chart-type">Chart type</Label>
                    <select
                      id="chart-type"
                      className="ky-w-full ky-rounded-md ky-border ky-border-input ky-bg-background ky-px-3 ky-py-2 ky-text-sm focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring"
                      value={(draft.config.chartType as string | undefined) ?? ""}
                      onChange={(e) => upConfig({ chartType: e.target.value || undefined })}
                    >
                      <option value="">Auto</option>
                      <option value="line">Line</option>
                      <option value="bar">Bar</option>
                      <option value="scatter">Scatter</option>
                    </select>
                  </div>
                  <div className="ky-grid ky-grid-cols-2 ky-gap-2">
                    <div className="ky-space-y-1">
                      <Label htmlFor="chart-xcol">X column</Label>
                      <Input
                        id="chart-xcol"
                        value={(draft.config.xCol as string | undefined) ?? ""}
                        onChange={(e) => upConfig({ xCol: e.target.value || undefined })}
                        placeholder="auto"
                      />
                    </div>
                    <div className="ky-space-y-1">
                      <Label htmlFor="chart-ycol">Y column</Label>
                      <Input
                        id="chart-ycol"
                        value={(draft.config.yCol as string | undefined) ?? ""}
                        onChange={(e) => upConfig({ yCol: e.target.value || undefined })}
                        placeholder="auto"
                      />
                    </div>
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Test query button + preview */}
          {!isMarkdown && (
            <div className="ky-space-y-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={handlePreview}
                disabled={!draft.query?.trim()}
              >
                <Play className="ky-mr-1.5 ky-h-3.5 ky-w-3.5" /> Test query
              </Button>
              {preview && (
                <div
                  key={previewKey}
                  className="ky-h-48 ky-rounded-md ky-border ky-bg-muted/10 ky-overflow-hidden"
                >
                  <PanelPreview panel={preview} />
                </div>
              )}
            </div>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button onClick={handleSubmit} disabled={!draft.title.trim()}>
            {initial ? "Save changes" : "Add panel"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function PanelPreview({ panel }: { panel: DashboardPanel }) {
  const timeRange = { preset: "1h" as const };
  switch (panel.panel_type) {
    case "chart":
      return <ChartPanelViz panel={panel} timeRange={timeRange} />;
    case "table":
      return <TablePanelViz panel={panel} timeRange={timeRange} />;
    case "stat":
      return <StatPanelViz panel={panel} timeRange={timeRange} />;
    default:
      return null;
  }
}
