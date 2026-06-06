import { beforeEach, expect, test, vi } from "vitest";
import {
  listDashboards,
  getDashboard,
  createDashboard,
  updateDashboard,
  deleteDashboard,
  type Dashboard,
  type DashboardWithPanels,
} from "./dashboards";
import { createTransport } from "./transport";

const mockFetch = vi.fn();
beforeEach(() => {
  mockFetch.mockReset();
});

function makeTransport(fetchMock: typeof fetch) {
  return createTransport({ endpoint: "http://localhost:7070", auth: { token: "tok" }, fetch: fetchMock });
}

const BASE_DASHBOARD: Dashboard = {
  id: "d1",
  name: "My Dashboard",
  description: null,
  time_range_preset: "1h",
  refresh_interval_seconds: null,
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
};

const DASHBOARD_WITH_PANELS: DashboardWithPanels = {
  ...BASE_DASHBOARD,
  panels: [],
};

// ── listDashboards ─────────────────────────────────────────────────────────

test("listDashboards fetches and returns array", async () => {
  mockFetch.mockResolvedValue(
    new Response(JSON.stringify([BASE_DASHBOARD]), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
  const result = await listDashboards(makeTransport(mockFetch));
  expect(result).toEqual([BASE_DASHBOARD]);
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/dashboards");
  expect(init.headers.get("authorization")).toBe("Bearer tok");
});

test("listDashboards throws unauthorized on 401", async () => {
  mockFetch.mockResolvedValue(new Response("nope", { status: 401 }));
  await expect(listDashboards(makeTransport(mockFetch))).rejects.toThrow(/unauthorized|token rejected/i);
});

// ── getDashboard ───────────────────────────────────────────────────────────

test("getDashboard fetches by id", async () => {
  mockFetch.mockResolvedValue(
    new Response(JSON.stringify(DASHBOARD_WITH_PANELS), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
  const result = await getDashboard(makeTransport(mockFetch), { id: "d1" });
  expect(result).toEqual(DASHBOARD_WITH_PANELS);
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/dashboards/d1");
});

test("getDashboard throws not found on 404", async () => {
  mockFetch.mockResolvedValue(new Response("not found", { status: 404 }));
  await expect(getDashboard(makeTransport(mockFetch), { id: "missing" })).rejects.toThrow(
    /not found/i,
  );
});

// ── createDashboard ────────────────────────────────────────────────────────

test("createDashboard POSTs and returns DashboardWithPanels", async () => {
  mockFetch.mockResolvedValue(
    new Response(JSON.stringify(DASHBOARD_WITH_PANELS), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
  const result = await createDashboard(makeTransport(mockFetch), { body: { name: "My Dashboard" } });
  expect(result).toEqual(DASHBOARD_WITH_PANELS);
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/dashboards");
  expect(init.method).toBe("POST");
  expect(JSON.parse(init.body as string)).toEqual({ name: "My Dashboard" });
});

// ── updateDashboard ────────────────────────────────────────────────────────

test("updateDashboard PATCHes by id", async () => {
  const updated: DashboardWithPanels = { ...DASHBOARD_WITH_PANELS, name: "Renamed" };
  mockFetch.mockResolvedValue(
    new Response(JSON.stringify(updated), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
  const result = await updateDashboard(makeTransport(mockFetch), { id: "d1", patch: { name: "Renamed" } });
  expect(result.name).toBe("Renamed");
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/dashboards/d1");
  expect(init.method).toBe("PATCH");
});

// ── deleteDashboard ────────────────────────────────────────────────────────

test("deleteDashboard sends DELETE and resolves void", async () => {
  mockFetch.mockResolvedValue(new Response(null, { status: 204 }));
  await expect(
    deleteDashboard(makeTransport(mockFetch), { id: "d1" }),
  ).resolves.toBeUndefined();
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/dashboards/d1");
  expect(init.method).toBe("DELETE");
});

test("deleteDashboard throws unauthorized on 401", async () => {
  mockFetch.mockResolvedValue(new Response("nope", { status: 401 }));
  await expect(deleteDashboard(makeTransport(mockFetch), { id: "d1" })).rejects.toThrow(
    /unauthorized|token rejected/i,
  );
});
