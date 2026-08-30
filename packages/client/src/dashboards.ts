import type { PensieveTransport } from "./transport";
import { errorFromResponse } from "./errors";

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Dashboard {
  id: string;
  name: string;
  description: string | null;
  time_range_preset: string;
  refresh_interval_seconds: number | null;
  created_at: string;
  updated_at: string;
}

export interface DashboardPanel {
  id: string;
  dashboard_id: string;
  title: string;
  panel_type: "chart" | "table" | "markdown" | "stat";
  query: string | null;
  database_name: string | null;
  config: Record<string, unknown>;
  grid_x: number;
  grid_y: number;
  grid_w: number;
  grid_h: number;
  display_order: number;
}

export type DashboardWithPanels = Dashboard & { panels: DashboardPanel[] };

export interface CreateDashboardBody {
  name: string;
  description?: string;
  panels?: Partial<DashboardPanel>[];
}

export interface DashboardUpdate {
  name?: string;
  description?: string | null;
  time_range_preset?: string;
  refresh_interval_seconds?: number | null;
  panels?: Partial<DashboardPanel>[];
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) throw await errorFromResponse(res);
  return res.json() as Promise<T>;
}

// ── API functions ─────────────────────────────────────────────────────────────

export async function listDashboards(t: PensieveTransport): Promise<Dashboard[]> {
  return handleResponse<Dashboard[]>(await t.request("/v1/dashboards"));
}

export async function getDashboard(
  t: PensieveTransport,
  args: { id: string },
): Promise<DashboardWithPanels> {
  return handleResponse<DashboardWithPanels>(await t.request(`/v1/dashboards/${args.id}`));
}

export async function createDashboard(
  t: PensieveTransport,
  args: { body: CreateDashboardBody },
): Promise<DashboardWithPanels> {
  return handleResponse<DashboardWithPanels>(
    await t.request("/v1/dashboards", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.body),
    }),
  );
}

export async function updateDashboard(
  t: PensieveTransport,
  args: { id: string; patch: DashboardUpdate },
): Promise<DashboardWithPanels> {
  return handleResponse<DashboardWithPanels>(
    await t.request(`/v1/dashboards/${args.id}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args.patch),
    }),
  );
}

export async function deleteDashboard(t: PensieveTransport, args: { id: string }): Promise<void> {
  const res = await t.request(`/v1/dashboards/${args.id}`, { method: "DELETE" });
  if (!res.ok) throw await errorFromResponse(res);
}
