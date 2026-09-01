/**
 * PensieveDashboard — self-contained embeddable dashboard viewer/editor.
 *
 * Renders a dashboard by ID using react-grid-layout for the panel grid.
 * Read-only by default; editable=true enables panel add/edit/delete and
 * layout drag-resize, with saves going directly to the client API.
 *
 * Decoupling notes vs web's _app.dashboards.$id.tsx:
 *   - Session (endpoint/token/db) → usePensieveClient() from PensieveProvider context
 *   - Router navigation → onEditChange prop (no tanstack-router dependency)
 *   - Toast (sonner) → dropped; save result is surfaced via onSaveSuccess/onSaveError
 *   - useSession.getState() Zustand calls → usePensieveClient() client object
 *   - database prop: scoped client via client.withDatabase(database). The outer
 *     PensieveDashboard wrapper overrides PensieveContext.client so ALL child hooks
 *     (usePensieveDashboards/getDashboard, usePanelQuery, AddPanelModal schema fetch)
 *     automatically use x-database: <database>. New panels also receive
 *     database_name seeded from the prop so their stored database is correct.
 *
 * Auto-refresh: ticks every refresh_interval_seconds, invalidates the React
 * Query dashboard detail cache (same key used by usePensieveDashboards).
 *
 * Grid: react-grid-layout with ResizeObserver for responsive container width.
 * draggableHandle targets .pensieve-panel-drag-handle (set in PanelCard).
 */

import React, {
  useState,
  useEffect,
  useRef,
  useCallback,
} from "react";
import GridLayout, { type Layout } from "react-grid-layout";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Plus, Eye, Pencil, RefreshCw, Save } from "lucide-react";
import type { DashboardWithPanels, DashboardPanel } from "@pensieve-ai/client";
import { usePensieveClient, usePensieveContext, PensieveContext } from "../provider/context";
import { usePensieveDashboards } from "../hooks/usePensieveDashboards";
import { Button } from "../internal/ui/button";
import { PanelCard } from "./PanelCard";
import { AddPanelModal, type PanelDraft } from "./AddPanelModal";
import type { TimeRange, TimeRangePreset } from "../query/time-range/time-range-types";

// ── Public types ──────────────────────────────────────────────────────────────

export type { TimeRange };

export interface PensieveDashboardProps {
  /** ID of the dashboard to load. */
  dashboardId: string;
  /**
   * Allow panel add/edit/delete and drag-to-reorder.
   * Default false (read-only).
   */
  editable?: boolean;
  /**
   * Override the dashboard's stored time range.
   * When not provided, the dashboard's time_range_preset from the server is used.
   */
  timeRange?: TimeRange;
  /**
   * Override the default Pensieve database for ALL panel queries and dashboard
   * loads. When set, PensieveDashboard calls client.withDatabase(database) to
   * create a scoped client view and overrides the PensieveContext for all child
   * components, so every request (dashboard load, panel query, schema fetch)
   * carries x-database: <database>. New panels are also pre-seeded with
   * this database_name so their stored value matches.
   */
  database?: string;
  /** Additional CSS class on the root element. */
  className?: string;
  /** Inline style on the root element. */
  style?: React.CSSProperties;
  /** Content rendered while the dashboard is loading. */
  fallback?: React.ReactNode;
  /**
   * Called when a panel title bar is clicked (read-only mode only).
   * Not called in editable mode (clicks go to edit/delete buttons).
   */
  onPanelClick?: (panelId: string) => void;
  /**
   * Called when editable mode is toggled (allows the host to control the URL
   * or other state instead of relying on internal state).
   * If not provided, edit mode is managed internally.
   */
  onEditChange?: (editable: boolean) => void;
  /**
   * Called after a successful save. Receives the updated dashboard.
   */
  onSaveSuccess?: (dashboard: DashboardWithPanels) => void;
  /**
   * Called when a save fails.
   */
  onSaveError?: (err: unknown) => void;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function panelsToLayout(panels: DashboardPanel[]): Layout[] {
  return panels.map((p) => ({
    i: p.id,
    x: p.grid_x,
    y: p.grid_y,
    w: p.grid_w,
    h: p.grid_h,
    minH: 2,
    minW: 2,
  }));
}

function applyLayout(panels: DashboardPanel[], layout: Layout[]): DashboardPanel[] {
  return panels.map((p) => {
    const l = layout.find((lItem) => lItem.i === p.id);
    if (!l) return p;
    return { ...p, grid_x: l.x, grid_y: l.y, grid_w: l.w, grid_h: l.h };
  });
}

function deepEqual(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

const PRESETS: { value: TimeRangePreset; label: string }[] = [
  { value: "5m", label: "5 min" },
  { value: "15m", label: "15 min" },
  { value: "1h", label: "1 hour" },
  { value: "6h", label: "6 hours" },
  { value: "24h", label: "24 hours" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
];

// ── TimeRangeSelect ───────────────────────────────────────────────────────────

function TimeRangeSelect({
  value,
  onChange,
}: {
  value: TimeRange;
  onChange: (r: TimeRange) => void;
}) {
  return (
    <select
      className="pv-h-7 pv-rounded-md pv-border pv-border-input pv-bg-background pv-px-2 pv-text-xs focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring"
      value={value.preset}
      onChange={(e) => onChange({ preset: e.target.value as TimeRangePreset })}
    >
      {PRESETS.map((p) => (
        <option key={p.value} value={p.value}>
          Last {p.label}
        </option>
      ))}
    </select>
  );
}

// ── PensieveDashboard ─────────────────────────────────────────────────────────────

/** Public entry point — wraps inner in a scoped client context when database is set. */
export function PensieveDashboard({ database, ...rest }: PensieveDashboardProps) {
  const ctx = usePensieveContext();
  const scopedCtx = database ? { ...ctx, client: ctx.client.withDatabase(database) } : ctx;
  return (
    <PensieveContext.Provider value={scopedCtx}>
      <PensieveDashboardInner database={database} {...rest} />
    </PensieveContext.Provider>
  );
}

function PensieveDashboardInner({
  dashboardId,
  editable: editableProp = false,
  timeRange: timeRangeProp,
  database,
  className,
  style,
  fallback,
  onPanelClick,
  onEditChange,
  onSaveSuccess,
  onSaveError,
}: PensieveDashboardProps) {
  const client = usePensieveClient();
  const { getDashboard } = usePensieveDashboards();
  const queryClient = useQueryClient();
  const endpoint = client.transport.endpoint;
  const dbKey = client.transport.database ?? "default";

  // ── load dashboard ───────────────────────────────────────────────────────
  const [fetched, setFetched] = useState<DashboardWithPanels | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [fetchError, setFetchError] = useState<unknown>(null);

  useEffect(() => {
    let cancelled = false;
    setIsLoading(true);
    setFetchError(null);
    getDashboard(dashboardId)
      .then((d) => {
        if (!cancelled) {
          setFetched(d);
          setIsLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setFetchError(err);
          setIsLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [dashboardId, getDashboard]);

  // ── draft state (editable mode) ──────────────────────────────────────────
  const [draft, setDraft] = useState<DashboardWithPanels | null>(null);

  useEffect(() => {
    if (!fetched) return;
    setDraft((prev) => {
      if (prev === null) return fetched;
      // Only reset draft if not dirty
      const currentDirty = !deepEqual(prev, fetched);
      if (!currentDirty) return fetched;
      return prev;
    });
  }, [fetched]);

  const dirty = draft !== null && fetched !== null && !deepEqual(draft, fetched);

  // ── edit mode (internal unless onEditChange provided) ────────────────────
  const [internalEditable, setInternalEditable] = useState(editableProp);
  const editMode = onEditChange ? editableProp : internalEditable;

  const toggleEditMode = () => {
    const next = !editMode;
    if (onEditChange) {
      onEditChange(next);
    } else {
      setInternalEditable(next);
    }
  };

  // ── name editing ─────────────────────────────────────────────────────────
  const [editingName, setEditingName] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);

  // ── time range ────────────────────────────────────────────────────────────
  const [internalTimeRange, setInternalTimeRange] = useState<TimeRange>({ preset: "1h" });
  useEffect(() => {
    if (fetched?.time_range_preset) {
      setInternalTimeRange({ preset: fetched.time_range_preset as TimeRangePreset });
    }
  }, [fetched?.time_range_preset]);
  const timeRange = timeRangeProp ?? internalTimeRange;

  // ── auto-refresh ──────────────────────────────────────────────────────────
  const autoRefreshRef = useRef<ReturnType<typeof setInterval> | null>(null);
  useEffect(() => {
    if (autoRefreshRef.current) clearInterval(autoRefreshRef.current);
    const interval = fetched?.refresh_interval_seconds;
    if (!editMode && interval && interval > 0) {
      autoRefreshRef.current = setInterval(() => {
        void queryClient.invalidateQueries({
          queryKey: ["pensieve", endpoint, dbKey, "dashboards", dashboardId],
        });
      }, interval * 1000);
    }
    return () => {
      if (autoRefreshRef.current) clearInterval(autoRefreshRef.current);
    };
  }, [editMode, fetched?.refresh_interval_seconds, dashboardId, endpoint, dbKey, queryClient]);

  // ── panel edit state ──────────────────────────────────────────────────────
  const [addPanelOpen, setAddPanelOpen] = useState(false);
  const [editingPanel, setEditingPanel] = useState<DashboardPanel | null>(null);

  const handleAddPanel = useCallback(
    (panelDraft: PanelDraft) => {
      if (!draft) return;
      const maxY = draft.panels.reduce((m, p) => Math.max(m, p.grid_y + p.grid_h), 0);
      const newPanel: DashboardPanel = {
        id: crypto.randomUUID(),
        dashboard_id: dashboardId,
        ...panelDraft,
        grid_x: 0,
        grid_y: maxY,
        grid_w: 6,
        grid_h: 4,
        display_order: draft.panels.length,
      };
      setDraft({ ...draft, panels: [...draft.panels, newPanel] });
    },
    [draft, dashboardId],
  );

  const handleEditPanel = useCallback(
    (panelDraft: PanelDraft) => {
      if (!draft || !editingPanel) return;
      setDraft({
        ...draft,
        panels: draft.panels.map((p) =>
          p.id === editingPanel.id ? { ...p, ...panelDraft } : p,
        ),
      });
      setEditingPanel(null);
    },
    [draft, editingPanel],
  );

  const handleDeletePanel = useCallback(
    (panelId: string) => {
      if (!draft) return;
      setDraft({ ...draft, panels: draft.panels.filter((p) => p.id !== panelId) });
    },
    [draft],
  );

  const handleLayoutChange = useCallback(
    (layout: Layout[]) => {
      if (!draft || !editMode) return;
      setDraft({ ...draft, panels: applyLayout(draft.panels, layout) });
    },
    [draft, editMode],
  );

  // ── save ──────────────────────────────────────────────────────────────────
  const { mutate: save, isPending: saving } = useMutation({
    mutationFn: async () => {
      if (!draft) throw new Error("No draft");
      return client.dashboards.updateDashboard({
        id: dashboardId,
        patch: {
          name: draft.name,
          description: draft.description,
          time_range_preset: timeRange.preset,
          panels: draft.panels.map((p, idx) => ({
            ...p,
            display_order: idx,
          })),
        },
      });
    },
    onSuccess: (saved) => {
      setDraft(saved);
      setFetched(saved);
      void queryClient.invalidateQueries({
        queryKey: ["pensieve", endpoint, dbKey, "dashboards"],
      });
      void queryClient.setQueryData(
        ["pensieve", endpoint, dbKey, "dashboards", dashboardId],
        saved,
      );
      onSaveSuccess?.(saved);
    },
    onError: (err) => {
      onSaveError?.(err);
    },
  });

  // ── grid width ────────────────────────────────────────────────────────────
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(1200);

  useEffect(() => {
    if (!containerRef.current) return;
    const obs = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w) setContainerWidth(w);
    });
    obs.observe(containerRef.current);
    return () => obs.disconnect();
  }, []);

  // ── render states ─────────────────────────────────────────────────────────
  if (isLoading) {
    return (
      fallback ?? (
        <div className="pv-flex pv-h-full pv-items-center pv-justify-center pv-text-sm pv-text-muted-foreground pv-animate-pulse">
          Loading dashboard…
        </div>
      )
    );
  }

  if (fetchError || (!draft && !isLoading)) {
    return (
      <div className="pv-flex pv-h-full pv-items-center pv-justify-center">
        <p className="pv-text-sm pv-text-muted-foreground">Dashboard not found.</p>
      </div>
    );
  }

  if (!draft) return null;

  const layout = panelsToLayout(draft.panels);

  return (
    <div className={`pv-flex pv-h-full pv-flex-col pv-bg-muted/10 ${className ?? ""}`} style={style}>
      {/* ── Toolbar ──────────────────────────────────────────────────────── */}
      <div className="pv-flex pv-items-center pv-gap-2 pv-border-b pv-bg-background pv-px-4 pv-py-2 pv-shadow-sm">
        {/* Dashboard name */}
        {editingName ? (
          <input
            ref={nameRef}
            className="pv-h-7 pv-rounded-md pv-border pv-border-input pv-bg-background pv-px-2 pv-text-sm pv-font-medium focus-visible:pv-outline-none focus-visible:pv-ring-2 focus-visible:pv-ring-ring"
            defaultValue={draft.name}
            autoFocus
            onBlur={(e) => {
              const name = e.target.value.trim() || draft.name;
              setDraft({ ...draft, name });
              setEditingName(false);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === "Escape") {
                (e.target as HTMLInputElement).blur();
              }
            }}
          />
        ) : (
          <button
            className="pv-rounded pv-px-1 pv-text-sm pv-font-semibold hover:pv-bg-accent/50"
            onClick={() => editMode && setEditingName(true)}
            title={editMode ? "Click to rename" : undefined}
          >
            {draft.name}
          </button>
        )}

        {/* Time range */}
        <TimeRangeSelect value={timeRange} onChange={setInternalTimeRange} />

        {/* Auto-refresh indicator */}
        {fetched?.refresh_interval_seconds && !editMode && (
          <div className="pv-flex pv-items-center pv-gap-1 pv-text-xs pv-text-muted-foreground">
            <RefreshCw
              className="pv-h-3 pv-w-3 pv-animate-spin"
              style={{ animationDuration: "3s" }}
            />
            {fetched.refresh_interval_seconds}s
          </div>
        )}

        <div className="pv-ml-auto pv-flex pv-items-center pv-gap-1.5">
          {/* Add panel (edit only) */}
          {editMode && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => setAddPanelOpen(true)}
              data-testid="add-panel-btn"
            >
              <Plus className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> Add panel
            </Button>
          )}

          {/* View/Edit toggle */}
          <Button size="sm" variant="outline" onClick={toggleEditMode} data-testid="edit-toggle-btn">
            {editMode ? (
              <>
                <Eye className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> View
              </>
            ) : (
              <>
                <Pencil className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> Edit
              </>
            )}
          </Button>

          {/* Save (edit only) */}
          {editMode && (
            <Button
              size="sm"
              onClick={() => save()}
              disabled={!dirty || saving}
              data-testid="save-btn"
            >
              <Save className="pv-mr-1 pv-h-3.5 pv-w-3.5" />
              {saving ? "Saving…" : "Save"}
            </Button>
          )}
        </div>
      </div>

      {/* ── Grid ─────────────────────────────────────────────────────────── */}
      <div ref={containerRef} className="pv-flex-1 pv-overflow-y-auto pv-p-4">
        {draft.panels.length === 0 && (
          <div className="pv-flex pv-h-64 pv-flex-col pv-items-center pv-justify-center pv-gap-3 pv-rounded-lg pv-border-2 pv-border-dashed pv-border-muted-foreground/20 pv-text-muted-foreground">
            <p className="pv-text-sm">No panels yet.</p>
            {editMode && (
              <Button size="sm" variant="outline" onClick={() => setAddPanelOpen(true)}>
                <Plus className="pv-mr-1 pv-h-3.5 pv-w-3.5" /> Add your first panel
              </Button>
            )}
          </div>
        )}

        {draft.panels.length > 0 && (
          <GridLayout
            className="layout"
            layout={layout}
            cols={12}
            rowHeight={40}
            width={containerWidth}
            isDraggable={editMode}
            isResizable={editMode}
            draggableHandle=".pensieve-panel-drag-handle"
            onLayoutChange={handleLayoutChange}
            margin={[8, 8]}
            containerPadding={[0, 0]}
          >
            {draft.panels.map((panel) => (
              <div key={panel.id}>
                <PanelCard
                  panel={panel}
                  timeRange={timeRange}
                  editable={editMode}
                  onDelete={handleDeletePanel}
                  onEdit={(p) => {
                    setEditingPanel(p);
                    setAddPanelOpen(true);
                  }}
                  onPanelClick={!editMode ? onPanelClick : undefined}
                />
              </div>
            ))}
          </GridLayout>
        )}
      </div>

      {/* ── Add / Edit panel modal ────────────────────────────────────────── */}
      {editMode && (
        <AddPanelModal
          open={addPanelOpen}
          onClose={() => {
            setAddPanelOpen(false);
            setEditingPanel(null);
          }}
          onSave={editingPanel ? handleEditPanel : handleAddPanel}
          initial={editingPanel}
          defaultDatabase={database}
        />
      )}
    </div>
  );
}
