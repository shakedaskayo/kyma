/**
 * usePensieveDashboards tests.
 */
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, cleanup, waitFor } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { PensieveProvider } from "../provider/PensieveProvider";
import { usePensieveDashboards } from "./usePensieveDashboards";
import type { Dashboard, DashboardWithPanels } from "@pensieve-ai/client";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function makeQC() {
  return new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
}

function wrapper(qc: QueryClient) {
  return ({ children }: { children: React.ReactNode }) => (
    <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={qc}>
      {children}
    </PensieveProvider>
  );
}

function jsonResponse(body: unknown) {
  return Promise.resolve(
    new Response(JSON.stringify(body), {
      status: 200,
      headers: new Headers({ "content-type": "application/json" }),
    }),
  );
}

const DASHBOARDS: Dashboard[] = [
  {
    id: "d1",
    name: "Overview",
    description: null,
    time_range_preset: "last_1h",
    refresh_interval_seconds: null,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    id: "d2",
    name: "Errors",
    description: "Error tracking",
    time_range_preset: "last_24h",
    refresh_interval_seconds: 60,
    created_at: "2026-01-02T00:00:00Z",
    updated_at: "2026-01-02T00:00:00Z",
  },
];

const DASHBOARD_WITH_PANELS: DashboardWithPanels = {
  ...DASHBOARDS[0],
  panels: [
    {
      id: "p1",
      dashboard_id: "d1",
      title: "Request rate",
      panel_type: "chart",
      query: "events | count",
      database_name: "db1",
      config: {},
      grid_x: 0,
      grid_y: 0,
      grid_w: 6,
      grid_h: 4,
      display_order: 0,
    },
  ],
};

describe("usePensieveDashboards", () => {
  it("loads and returns dashboards list", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => jsonResponse(DASHBOARDS)),
    );

    const { result } = renderHook(() => usePensieveDashboards(), { wrapper: wrapper(qc) });

    expect(result.current.isLoading).toBe(true);

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });

    expect(result.current.dashboards).toHaveLength(2);
    expect(result.current.dashboards[0].id).toBe("d1");
    expect(result.current.error).toBeNull();
  });

  it("surfaces error on fetch failure", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response("not found", { status: 404 })),
    );

    const { result } = renderHook(() => usePensieveDashboards(), { wrapper: wrapper(qc) });

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });

    expect(result.current.error).toBeTruthy();
    expect(result.current.dashboards).toEqual([]);
  });

  it("getDashboard returns a dashboard with panels", async () => {
    const qc = makeQC();
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation((url: string) => {
        if (url.includes("/v1/dashboards/d1")) return jsonResponse(DASHBOARD_WITH_PANELS);
        return jsonResponse(DASHBOARDS);
      }),
    );

    const { result } = renderHook(() => usePensieveDashboards(), { wrapper: wrapper(qc) });

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });

    const d = await result.current.getDashboard("d1");
    expect(d.id).toBe("d1");
    expect(d.panels).toHaveLength(1);
    expect(d.panels[0].title).toBe("Request rate");
  });

  it("refetch triggers re-fetch from server", async () => {
    const qc = makeQC();
    let call = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(() => {
        call++;
        return jsonResponse(call === 1 ? DASHBOARDS.slice(0, 1) : DASHBOARDS);
      }),
    );

    const { result } = renderHook(() => usePensieveDashboards(), { wrapper: wrapper(qc) });

    await waitFor(() => expect(result.current.isLoading).toBe(false), { timeout: 3000 });
    expect(result.current.dashboards).toHaveLength(1);

    result.current.refetch();

    await waitFor(() => expect(result.current.dashboards).toHaveLength(2), { timeout: 3000 });
  });
});
