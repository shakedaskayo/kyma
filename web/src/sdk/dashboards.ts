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

type BaseArgs = { endpoint: string; token: string };

function headers(token: string): Record<string, string> {
  return { authorization: `Bearer ${token}`, "content-type": "application/json" };
}

async function handleResponse<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) {
    throw new Error(`unauthorized (${res.status})`);
  }
  if (res.status === 404) {
    throw new Error("not found");
  }
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

function base(endpoint: string) {
  return endpoint.replace(/\/$/, "");
}

// ── API functions ─────────────────────────────────────────────────────────────

export async function listDashboards(args: BaseArgs): Promise<Dashboard[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/dashboards`, {
    headers: headers(args.token),
  });
  return handleResponse<Dashboard[]>(res);
}

export async function getDashboard(
  args: BaseArgs & { id: string },
): Promise<DashboardWithPanels> {
  const res = await fetch(`${base(args.endpoint)}/v1/dashboards/${args.id}`, {
    headers: headers(args.token),
  });
  return handleResponse<DashboardWithPanels>(res);
}

export async function createDashboard(
  args: BaseArgs & { body: CreateDashboardBody },
): Promise<DashboardWithPanels> {
  const res = await fetch(`${base(args.endpoint)}/v1/dashboards`, {
    method: "POST",
    headers: headers(args.token),
    body: JSON.stringify(args.body),
  });
  return handleResponse<DashboardWithPanels>(res);
}

export async function updateDashboard(
  args: BaseArgs & { id: string; patch: DashboardUpdate },
): Promise<DashboardWithPanels> {
  const res = await fetch(`${base(args.endpoint)}/v1/dashboards/${args.id}`, {
    method: "PATCH",
    headers: headers(args.token),
    body: JSON.stringify(args.patch),
  });
  return handleResponse<DashboardWithPanels>(res);
}

export async function deleteDashboard(args: BaseArgs & { id: string }): Promise<void> {
  const res = await fetch(`${base(args.endpoint)}/v1/dashboards/${args.id}`, {
    method: "DELETE",
    headers: headers(args.token),
  });
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
}
