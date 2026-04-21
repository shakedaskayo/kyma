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

const mockFetch = vi.fn();
beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
});

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
  const result = await listDashboards({ endpoint: "http://localhost:7070", token: "tok" });
  expect(result).toEqual([BASE_DASHBOARD]);
  expect(mockFetch).toHaveBeenCalledWith(
    "http://localhost:7070/v1/dashboards",
    expect.objectContaining({
      headers: expect.objectContaining({ authorization: "Bearer tok" }),
    }),
  );
});

test("listDashboards throws unauthorized on 401", async () => {
  mockFetch.mockResolvedValue(new Response("nope", { status: 401 }));
  await expect(listDashboards({ endpoint: "http://x", token: "t" })).rejects.toThrow(/unauthorized/i);
});

// ── getDashboard ───────────────────────────────────────────────────────────

test("getDashboard fetches by id", async () => {
  mockFetch.mockResolvedValue(
    new Response(JSON.stringify(DASHBOARD_WITH_PANELS), {
      status: 200,
      headers: { "content-type": "application/json" },
    }),
  );
  const result = await getDashboard({ endpoint: "http://localhost:7070", token: "tok", id: "d1" });
  expect(result).toEqual(DASHBOARD_WITH_PANELS);
  expect(mockFetch).toHaveBeenCalledWith(
    "http://localhost:7070/v1/dashboards/d1",
    expect.any(Object),
  );
});

test("getDashboard throws not found on 404", async () => {
  mockFetch.mockResolvedValue(new Response("not found", { status: 404 }));
  await expect(getDashboard({ endpoint: "http://x", token: "t", id: "missing" })).rejects.toThrow(
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
  const result = await createDashboard({
    endpoint: "http://localhost:7070",
    token: "tok",
    body: { name: "My Dashboard" },
  });
  expect(result).toEqual(DASHBOARD_WITH_PANELS);
  const call = mockFetch.mock.calls[0];
  expect(call[0]).toBe("http://localhost:7070/v1/dashboards");
  expect(call[1].method).toBe("POST");
  expect(JSON.parse(call[1].body as string)).toEqual({ name: "My Dashboard" });
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
  const result = await updateDashboard({
    endpoint: "http://localhost:7070",
    token: "tok",
    id: "d1",
    patch: { name: "Renamed" },
  });
  expect(result.name).toBe("Renamed");
  const call = mockFetch.mock.calls[0];
  expect(call[0]).toBe("http://localhost:7070/v1/dashboards/d1");
  expect(call[1].method).toBe("PATCH");
});

// ── deleteDashboard ────────────────────────────────────────────────────────

test("deleteDashboard sends DELETE and resolves void", async () => {
  mockFetch.mockResolvedValue(new Response(null, { status: 204 }));
  await expect(
    deleteDashboard({ endpoint: "http://localhost:7070", token: "tok", id: "d1" }),
  ).resolves.toBeUndefined();
  expect(mockFetch).toHaveBeenCalledWith(
    "http://localhost:7070/v1/dashboards/d1",
    expect.objectContaining({ method: "DELETE" }),
  );
});

test("deleteDashboard throws unauthorized on 401", async () => {
  mockFetch.mockResolvedValue(new Response("nope", { status: 401 }));
  await expect(deleteDashboard({ endpoint: "http://x", token: "t", id: "d1" })).rejects.toThrow(
    /unauthorized/i,
  );
});
