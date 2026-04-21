import { createFileRoute, useNavigate, useSearch, Link } from "@tanstack/react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useState, useEffect, useRef, useCallback } from "react";
import GridLayout, { type Layout } from "react-grid-layout";
import {
  ArrowLeft,
  Share2,
  Save,
  Plus,
  Eye,
  Pencil,
  RefreshCw,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";
import { useSession } from "@/sdk/session";
import {
  getDashboard,
  updateDashboard,
  type DashboardWithPanels,
  type DashboardPanel,
} from "@/sdk/dashboards";
import { PanelCard } from "@/features/dashboards/PanelCard";
import { AddPanelModal, type PanelDraft } from "@/features/dashboards/AddPanelModal";
import type { TimeRange } from "@/features/tabs/workspace-store";

export const Route = createFileRoute("/_app/dashboards/$id")({
  validateSearch: (s: Record<string, unknown>) => ({
    edit: typeof s.edit === "string" ? s.edit : undefined,
  }),
  component: DashboardPage,
});

// ── helpers ───────────────────────────────────────────────────────────────────

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

// ── component ─────────────────────────────────────────────────────────────────

function DashboardPage() {
  const { id } = Route.useParams();
  const search = useSearch({ from: "/_app/dashboards/$id" });
  const navigate = useNavigate();
  const { endpoint, token } = useSession();
  const qc = useQueryClient();

  const editMode = search.edit === "1";

  const { data: fetched, isLoading } = useQuery({
    queryKey: ["dashboard", id, endpoint],
    queryFn: () => getDashboard({ endpoint, token, id }),
    staleTime: 30_000,
    enabled: Boolean(endpoint && token && id),
  });

  // ── draft state ──────────────────────────────────────────────────────────────
  const [draft, setDraft] = useState<DashboardWithPanels | null>(null);

  // Seed draft from fetched data (once, or when fetched changes and not dirty)
  useEffect(() => {
    if (!fetched) return;
    setDraft((prev) => {
      if (prev === null) return fetched;
      if (!dirty) return fetched;
      return prev;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fetched]);

  const dirty = draft !== null && fetched !== null && !deepEqual(draft, fetched);

  // Inline name editing
  const [editingName, setEditingName] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);

  // Time range — local to this dashboard
  const [timeRange, setTimeRange] = useState<TimeRange>({ preset: "1h" });
  useEffect(() => {
    if (fetched?.time_range_preset) {
      setTimeRange({ preset: fetched.time_range_preset as TimeRange["preset"] });
    }
  }, [fetched?.time_range_preset]);

  // Auto-refresh
  const autoRefreshRef = useRef<ReturnType<typeof setInterval> | null>(null);
  useEffect(() => {
    if (autoRefreshRef.current) clearInterval(autoRefreshRef.current);
    const interval = fetched?.refresh_interval_seconds;
    if (!editMode && interval && interval > 0) {
      autoRefreshRef.current = setInterval(() => {
        void qc.invalidateQueries({ queryKey: ["dashboard", id, endpoint] });
      }, interval * 1000);
    }
    return () => {
      if (autoRefreshRef.current) clearInterval(autoRefreshRef.current);
    };
  }, [editMode, fetched?.refresh_interval_seconds, id, endpoint, qc]);

  // ── add/edit panel modal ─────────────────────────────────────────────────────
  const [addPanelOpen, setAddPanelOpen] = useState(false);
  const [editingPanel, setEditingPanel] = useState<DashboardPanel | null>(null);

  const handleAddPanel = useCallback(
    (panelDraft: PanelDraft) => {
      if (!draft) return;
      const maxY = draft.panels.reduce((m, p) => Math.max(m, p.grid_y + p.grid_h), 0);
      const newPanel: DashboardPanel = {
        id: crypto.randomUUID(),
        dashboard_id: id,
        ...panelDraft,
        grid_x: 0,
        grid_y: maxY,
        grid_w: 6,
        grid_h: 4,
        display_order: draft.panels.length,
      };
      setDraft({ ...draft, panels: [...draft.panels, newPanel] });
    },
    [draft, id],
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

  // ── save ─────────────────────────────────────────────────────────────────────
  const { mutate: save, isPending: saving } = useMutation({
    mutationFn: () => {
      if (!draft) throw new Error("No draft");
      return updateDashboard({
        endpoint,
        token,
        id,
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
      void qc.invalidateQueries({ queryKey: ["dashboards", endpoint] });
      void qc.setQueryData(["dashboard", id, endpoint], saved);
      toast.success("Dashboard saved.");
    },
    onError: (e) => {
      toast.error(`Save failed: ${(e as Error).message}`);
    },
  });

  // ── share ─────────────────────────────────────────────────────────────────────
  const handleShare = () => {
    const url = new URL(window.location.href);
    url.searchParams.set("edit", "0");
    void navigator.clipboard.writeText(url.toString()).then(() => {
      toast.success("Link copied — anyone with access can view this dashboard.");
    });
  };

  // ── toggle edit mode ─────────────────────────────────────────────────────────
  const toggleEditMode = () => {
    void navigate({
      to: "/dashboards/$id",
      params: { id },
      search: { edit: editMode ? "0" : "1" },
      replace: true,
    });
  };

  // ── container width for GridLayout ──────────────────────────────────────────
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

  // ── loading / error states ───────────────────────────────────────────────────
  if (isLoading) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground animate-pulse">
        Loading dashboard…
      </div>
    );
  }

  if (!draft && !isLoading) {
    return (
      <div className="flex h-full items-center justify-center">
        <p className="text-sm text-muted-foreground">Dashboard not found.</p>
      </div>
    );
  }

  if (!draft) return null;

  const layout = panelsToLayout(draft.panels);

  return (
    <div className="flex h-full flex-col bg-muted/10">
      {/* ── Toolbar ─────────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-2 border-b bg-background px-4 py-2 shadow-sm">
        {/* Back */}
        <Button variant="ghost" size="sm" asChild>
          <Link to="/dashboards">
            <ArrowLeft className="h-4 w-4" />
          </Link>
        </Button>

        {/* Dashboard name */}
        {editingName ? (
          <input
            ref={nameRef}
            className="h-7 rounded-md border border-input bg-background px-2 text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
            className="rounded px-1 text-sm font-semibold hover:bg-accent/50"
            onClick={() => editMode && setEditingName(true)}
            title={editMode ? "Click to rename" : undefined}
          >
            {draft.name}
          </button>
        )}

        {/* Time range */}
        <TimeRangeSelect value={timeRange} onChange={setTimeRange} />

        {/* Auto-refresh indicator */}
        {fetched?.refresh_interval_seconds && !editMode && (
          <div className="flex items-center gap-1 text-xs text-muted-foreground">
            <RefreshCw className="h-3 w-3 animate-spin" style={{ animationDuration: "3s" }} />
            {fetched.refresh_interval_seconds}s
          </div>
        )}

        <div className="ml-auto flex items-center gap-1.5">
          {/* Add panel (edit only) */}
          {editMode && (
            <Button size="sm" variant="outline" onClick={() => setAddPanelOpen(true)}>
              <Plus className="mr-1 h-3.5 w-3.5" /> Add panel
            </Button>
          )}

          {/* View/Edit toggle */}
          <Button size="sm" variant="outline" onClick={toggleEditMode}>
            {editMode ? (
              <><Eye className="mr-1 h-3.5 w-3.5" /> View</>
            ) : (
              <><Pencil className="mr-1 h-3.5 w-3.5" /> Edit</>
            )}
          </Button>

          {/* Share */}
          <Button size="sm" variant="outline" onClick={handleShare}>
            <Share2 className="mr-1 h-3.5 w-3.5" /> Share
          </Button>

          {/* Save (edit only) */}
          {editMode && (
            <Button size="sm" onClick={() => save()} disabled={!dirty || saving}>
              <Save className="mr-1 h-3.5 w-3.5" />
              {saving ? "Saving…" : "Save"}
            </Button>
          )}
        </div>
      </div>

      {/* ── Grid ───────────────────────────────────────────────────────────── */}
      <div ref={containerRef} className="flex-1 overflow-y-auto p-4">
        {draft.panels.length === 0 && (
          <div className="flex h-64 flex-col items-center justify-center gap-3 rounded-lg border-2 border-dashed border-muted-foreground/20 text-muted-foreground">
            <p className="text-sm">No panels yet.</p>
            {editMode && (
              <Button size="sm" variant="outline" onClick={() => setAddPanelOpen(true)}>
                <Plus className="mr-1 h-3.5 w-3.5" /> Add your first panel
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
            draggableHandle=".panel-drag-handle"
            onLayoutChange={handleLayoutChange}
            margin={[8, 8]}
            containerPadding={[0, 0]}
          >
            {draft.panels.map((panel) => (
              <div key={panel.id} className="panel-drag-handle">
                <PanelCard
                  panel={panel}
                  timeRange={timeRange}
                  editMode={editMode}
                  onDelete={handleDeletePanel}
                  onEdit={(p) => {
                    setEditingPanel(p);
                    setAddPanelOpen(true);
                  }}
                />
              </div>
            ))}
          </GridLayout>
        )}
      </div>

      {/* ── Add / Edit panel modal ──────────────────────────────────────────── */}
      <AddPanelModal
        open={addPanelOpen}
        onClose={() => {
          setAddPanelOpen(false);
          setEditingPanel(null);
        }}
        onSave={editingPanel ? handleEditPanel : handleAddPanel}
        initial={editingPanel}
      />
    </div>
  );
}

// ── Time range select ─────────────────────────────────────────────────────────

const PRESETS: { value: TimeRange["preset"]; label: string }[] = [
  { value: "5m", label: "5 min" },
  { value: "15m", label: "15 min" },
  { value: "1h", label: "1 hour" },
  { value: "6h", label: "6 hours" },
  { value: "24h", label: "24 hours" },
  { value: "7d", label: "7 days" },
  { value: "30d", label: "30 days" },
];

function TimeRangeSelect({
  value,
  onChange,
}: {
  value: TimeRange;
  onChange: (r: TimeRange) => void;
}) {
  return (
    <select
      className="h-7 rounded-md border border-input bg-background px-2 text-xs focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      value={value.preset}
      onChange={(e) => onChange({ preset: e.target.value as TimeRange["preset"] })}
    >
      {PRESETS.map((p) => (
        <option key={p.value} value={p.value}>
          Last {p.label}
        </option>
      ))}
    </select>
  );
}
