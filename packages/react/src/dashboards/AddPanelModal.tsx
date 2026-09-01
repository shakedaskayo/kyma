/**
 * AddPanelModal — create/edit dashboard panel form (editable mode only).
 *
 * Adapted from web's AddPanelModal with:
 *   - session → usePensieveClient() for catalog schema fetch
 *   - web shadcn components → internal/ui primitives (pv- prefix)
 *   - Toast dropped (no router-level notification infra in the SDK)
 *   - Preview uses SDK panel viz components
 */

import { useState } from "react";
import { ChevronDown, ChevronRight, Play } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import type { DashboardPanel } from "@pensieve-ai/client";
import { usePensieveClient } from "../provider/context";
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
  /** Default database_name for new panels (from PensieveDashboard.database prop). */
  defaultDatabase?: string;
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
      className="pv-h-28 pv-w-full pv-rounded-md pv-border pv-border-input pv-bg-background pv-p-2 pv-font-mono pv-text-xs placeholder:pv-text-muted-foreground focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring pv-resize-none"
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
      className="pv-text-sm pv-font-medium pv-leading-none peer-disabled:pv-cursor-not-allowed peer-disabled:pv-opacity-70"
    >
      {children}
    </label>
  );
}

// ── Component ─────────────────────────────────────────────────────────────────

export function AddPanelModal({ open, onClose, onSave, initial, defaultDatabase }: Props) {
  const client = usePensieveClient();
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
      : { ...DEFAULT_DRAFT, database_name: defaultDatabase ?? null },
  );

  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [preview, setPreview] = useState<DashboardPanel | null>(null);
  const [previewKey, setPreviewKey] = useState(0);

  const { data: schema } = useQuery({
    queryKey: ["pensieve", endpoint, database, "schema"],
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
    setDraft({ ...DEFAULT_DRAFT, database_name: defaultDatabase ?? null });
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
      <DialogContent className="pv-max-w-2xl pv-max-h-[90vh] pv-overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{initial ? "Edit panel" : "Add panel"}</DialogTitle>
        </DialogHeader>

        <div className="pv-space-y-4 pv-py-1">
          {/* Title */}
          <div className="pv-space-y-1">
            <Label htmlFor="panel-title">Title *</Label>
            <Input
              id="panel-title"
              value={draft.title}
              onChange={(e) => up({ title: e.target.value })}
              placeholder="Panel title"
            />
          </div>

          {/* Type */}
          <div className="pv-space-y-1">
            <Label>Type</Label>
            <div className="pv-flex pv-gap-2">
              {panelTypes.map((pt) => (
                <button
                  key={pt.value}
                  onClick={() => up({ panel_type: pt.value })}
                  className={`pv-rounded-md pv-border pv-px-3 pv-py-1.5 pv-text-xs pv-font-medium pv-transition ${
                    draft.panel_type === pt.value
                      ? "pv-border-primary pv-bg-primary pv-text-primary-foreground"
                      : "pv-border-border pv-text-muted-foreground hover:pv-border-primary/50 hover:pv-text-foreground"
                  }`}
                >
                  {pt.label}
                </button>
              ))}
            </div>
          </div>

          {/* Database */}
          {!isMarkdown && (
            <div className="pv-space-y-1">
              <Label htmlFor="panel-db">Database</Label>
              <select
                id="panel-db"
                className="pv-w-full pv-rounded-md pv-border pv-border-input pv-bg-background pv-px-3 pv-py-2 pv-text-sm focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring"
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
            <div className="pv-space-y-1">
              <Label>Query</Label>
              <MonacoMini value={draft.query ?? ""} onChange={(v) => up({ query: v })} />
            </div>
          )}

          {/* Markdown textarea */}
          {isMarkdown && (
            <div className="pv-space-y-1">
              <Label>Markdown content</Label>
              <textarea
                className="pv-h-40 pv-w-full pv-rounded-md pv-border pv-border-input pv-bg-background pv-p-2 pv-font-mono pv-text-xs focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring pv-resize-none"
                value={(draft.config.markdown as string | undefined) ?? ""}
                onChange={(e) => upConfig({ markdown: e.target.value })}
                placeholder="# My panel&#10;&#10;Write **markdown** here."
              />
            </div>
          )}

          {/* Table: max rows */}
          {isTable && (
            <div className="pv-space-y-1">
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
            <div className="pv-space-y-3">
              <div className="pv-space-y-1">
                <Label htmlFor="stat-label">Label</Label>
                <Input
                  id="stat-label"
                  value={(draft.config.label as string | undefined) ?? ""}
                  onChange={(e) => upConfig({ label: e.target.value })}
                  placeholder="e.g. requests/sec"
                />
              </div>
              <div className="pv-space-y-1">
                <Label htmlFor="stat-valuecol">Value column (optional)</Label>
                <Input
                  id="stat-valuecol"
                  value={(draft.config.valueCol as string | undefined) ?? ""}
                  onChange={(e) => upConfig({ valueCol: e.target.value || undefined })}
                  placeholder="auto-detect"
                />
              </div>
              <div className="pv-space-y-1">
                <Label htmlFor="stat-format">Format</Label>
                <select
                  id="stat-format"
                  className="pv-w-full pv-rounded-md pv-border pv-border-input pv-bg-background pv-px-3 pv-py-2 pv-text-sm focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring"
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
                className="pv-flex pv-items-center pv-gap-1 pv-text-xs pv-text-muted-foreground hover:pv-text-foreground"
                onClick={() => setAdvancedOpen((v) => !v)}
              >
                {advancedOpen ? (
                  <ChevronDown className="pv-h-3.5 pv-w-3.5" />
                ) : (
                  <ChevronRight className="pv-h-3.5 pv-w-3.5" />
                )}
                Advanced chart settings
              </button>
              {advancedOpen && (
                <div className="pv-mt-2 pv-space-y-3 pv-rounded-md pv-border pv-bg-muted/20 pv-p-3">
                  <div className="pv-space-y-1">
                    <Label htmlFor="chart-type">Chart type</Label>
                    <select
                      id="chart-type"
                      className="pv-w-full pv-rounded-md pv-border pv-border-input pv-bg-background pv-px-3 pv-py-2 pv-text-sm focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring"
                      value={(draft.config.chartType as string | undefined) ?? ""}
                      onChange={(e) => upConfig({ chartType: e.target.value || undefined })}
                    >
                      <option value="">Auto</option>
                      <option value="line">Line</option>
                      <option value="bar">Bar</option>
                      <option value="scatter">Scatter</option>
                    </select>
                  </div>
                  <div className="pv-grid pv-grid-cols-2 pv-gap-2">
                    <div className="pv-space-y-1">
                      <Label htmlFor="chart-xcol">X column</Label>
                      <Input
                        id="chart-xcol"
                        value={(draft.config.xCol as string | undefined) ?? ""}
                        onChange={(e) => upConfig({ xCol: e.target.value || undefined })}
                        placeholder="auto"
                      />
                    </div>
                    <div className="pv-space-y-1">
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
            <div className="pv-space-y-2">
              <Button
                type="button"
                size="sm"
                variant="outline"
                onClick={handlePreview}
                disabled={!draft.query?.trim()}
              >
                <Play className="pv-mr-1.5 pv-h-3.5 pv-w-3.5" /> Test query
              </Button>
              {preview && (
                <div
                  key={previewKey}
                  className="pv-h-48 pv-rounded-md pv-border pv-bg-muted/10 pv-overflow-hidden"
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
