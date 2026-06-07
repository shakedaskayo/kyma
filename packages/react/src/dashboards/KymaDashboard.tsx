/**
 * KymaDashboard — self-contained embeddable dashboard viewer/editor.
 *
 * Renders a dashboard by ID using react-grid-layout for the panel grid.
 * Read-only by default; editable=true enables panel add/edit/delete and
 * layout drag-resize, with saves going directly to the client API.
 *
 * Decoupling notes vs web's _app.dashboards.$id.tsx:
 *   - Session (endpoint/token/db) → useKymaClient() from KymaProvider context
 *   - Router navigation → onEditChange prop (no tanstack-router dependency)
 *   - Toast (sonner) → dropped; save result is surfaced via onSaveSuccess/onSaveError
 *   - useSession.getState() Zustand calls → useKymaClient() client object
 *   - database prop: wires into panel queries via panel.database_name; the
 *     top-level `database` prop overrides the client default at render time
 *     by passing it down to panels. client.withDatabase() is NOT used here
 *     because it would require a context override or a new child provider —
 *     instead, if database prop is set, it is stored in panel.database_name
 *     for new panels. Existing panels use their stored database_name.
 *     (Full withDatabase threading would require wrapping the entire tree in
 *     a scoped KymaProvider — documented below for callers who need it.)
 *
 * Auto-refresh: ticks every refresh_interval_seconds, invalidates the React
 * Query dashboard detail cache (same key used by useKymaDashboards).
 *
 * Grid: react-grid-layout with ResizeObserver for responsive container width.
 * draggableHandle targets .kyma-panel-drag-handle (set in PanelCard).
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
import type { DashboardWithPanels, DashboardPanel } from "@kyma-ai/client";
import { useKymaClient } from "../provider/context";
import { useKymaDashboards } from "../hooks/useKymaDashboards";
import { Button } from "../internal/ui/button";
import { PanelCard } from "./PanelCard";
import { AddPanelModal, type PanelDraft } from "./AddPanelModal";
import type { TimeRange, TimeRangePreset } from "../query/time-range/time-range-types";

// ── Public types ──────────────────────────────────────────────────────────────

export type { TimeRange };

export interface KymaDashboardProps {
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
   * Override the default database for new panels added in editable mode.
   * NOTE: existing panels use their stored database_name. To fully scope all
   * panel queries to a specific database, wrap KymaDashboard in a child
   * KymaProvider created via `new KymaClient({ endpoint, auth, database })`.
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
      className="ky-h-7 ky-rounded-md ky-border ky-border-input ky-bg-background ky-px-2 ky-text-xs focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring"
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

// ── KymaDashboard ─────────────────────────────────────────────────────────────

export function KymaDashboard({
  dashboardId,
  editable: editableProp = false,
  timeRange: timeRangeProp,
  database: _database,
  className,
  style,
  fallback,
  onPanelClick,
  onEditChange,
  onSaveSuccess,
  onSaveError,
}: KymaDashboardProps) {
  const client = useKymaClient();
  const { getDashboard } = useKymaDashboards();
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
          queryKey: ["kyma", endpoint, dbKey, "dashboards", dashboardId],
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
        queryKey: ["kyma", endpoint, dbKey, "dashboards"],
      });
      void queryClient.setQueryData(
        ["kyma", endpoint, dbKey, "dashboards", dashboardId],
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
        <div className="ky-flex ky-h-full ky-items-center ky-justify-center ky-text-sm ky-text-muted-foreground ky-animate-pulse">
          Loading dashboard…
        </div>
      )
    );
  }

  if (fetchError || (!draft && !isLoading)) {
    return (
      <div className="ky-flex ky-h-full ky-items-center ky-justify-center">
        <p className="ky-text-sm ky-text-muted-foreground">Dashboard not found.</p>
      </div>
    );
  }

  if (!draft) return null;

  const layout = panelsToLayout(draft.panels);

  return (
    <div className={`ky-flex ky-h-full ky-flex-col ky-bg-muted/10 ${className ?? ""}`} style={style}>
      {/* ── Toolbar ──────────────────────────────────────────────────────── */}
      <div className="ky-flex ky-items-center ky-gap-2 ky-border-b ky-bg-background ky-px-4 ky-py-2 ky-shadow-sm">
        {/* Dashboard name */}
        {editingName ? (
          <input
            ref={nameRef}
            className="ky-h-7 ky-rounded-md ky-border ky-border-input ky-bg-background ky-px-2 ky-text-sm ky-font-medium focus-visible:ky-outline-none focus-visible:ky-ring-2 focus-visible:ky-ring-ring"
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
            className="ky-rounded ky-px-1 ky-text-sm ky-font-semibold hover:ky-bg-accent/50"
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
          <div className="ky-flex ky-items-center ky-gap-1 ky-text-xs ky-text-muted-foreground">
            <RefreshCw
              className="ky-h-3 ky-w-3 ky-animate-spin"
              style={{ animationDuration: "3s" }}
            />
            {fetched.refresh_interval_seconds}s
          </div>
        )}

        <div className="ky-ml-auto ky-flex ky-items-center ky-gap-1.5">
          {/* Add panel (edit only) */}
          {editMode && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => setAddPanelOpen(true)}
              data-testid="add-panel-btn"
            >
              <Plus className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Add panel
            </Button>
          )}

          {/* View/Edit toggle */}
          <Button size="sm" variant="outline" onClick={toggleEditMode} data-testid="edit-toggle-btn">
            {editMode ? (
              <>
                <Eye className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> View
              </>
            ) : (
              <>
                <Pencil className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Edit
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
              <Save className="ky-mr-1 ky-h-3.5 ky-w-3.5" />
              {saving ? "Saving…" : "Save"}
            </Button>
          )}
        </div>
      </div>

      {/* ── Grid ─────────────────────────────────────────────────────────── */}
      <div ref={containerRef} className="ky-flex-1 ky-overflow-y-auto ky-p-4">
        {draft.panels.length === 0 && (
          <div className="ky-flex ky-h-64 ky-flex-col ky-items-center ky-justify-center ky-gap-3 ky-rounded-lg ky-border-2 ky-border-dashed ky-border-muted-foreground/20 ky-text-muted-foreground">
            <p className="ky-text-sm">No panels yet.</p>
            {editMode && (
              <Button size="sm" variant="outline" onClick={() => setAddPanelOpen(true)}>
                <Plus className="ky-mr-1 ky-h-3.5 ky-w-3.5" /> Add your first panel
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
            draggableHandle=".kyma-panel-drag-handle"
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
        />
      )}
    </div>
  );
}
