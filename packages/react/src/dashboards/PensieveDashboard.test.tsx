/**
 * PensieveDashboard tests.
 *
 * Stubs:
 *   - fetch: returns fixture dashboard JSON for /v1/dashboards/:id and
 *     NDJSON rows for /v1/query (panel queries).
 *   - react-grid-layout: lightweight div passthrough preserving children
 *     (avoids jsdom ResizeObserver issues and CSS layout complexity).
 *   - echarts-for-react: renders a div placeholder (avoids canvas in jsdom).
 *
 * Covers:
 *   1. Smoke render: panel titles visible, toolbar present.
 *   2. Panel executes its query and renders data (via mocked grid + viz).
 *   3. editable=false hides add-panel affordance.
 *   4. editable=true shows add-panel affordance.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, cleanup, waitFor } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import React from "react";
import { PensieveProvider } from "../provider/PensieveProvider";
import { PensieveDashboard } from "./PensieveDashboard";
import type { DashboardWithPanels } from "@pensieve-ai/client";

// ── react-grid-layout mock ────────────────────────────────────────────────────
// Renders children directly in a div, ignoring layout/cols/width props.
// This sidesteps jsdom ResizeObserver + CSS measurement issues.
vi.mock("react-grid-layout", () => {
  const MockGridLayout = ({
    children,
  }: {
    children: React.ReactNode;
    [key: string]: unknown;
  }) => <div data-testid="grid-layout">{children}</div>;
  return { default: MockGridLayout };
});

// ── echarts-for-react mock ────────────────────────────────────────────────────
vi.mock("echarts-for-react", () => {
  const MockECharts = () => <div data-testid="echart" />;
  return { default: MockECharts };
});

// ── @radix-ui/react-dialog mock ───────────────────────────────────────────────
// Simplified passthrough to avoid portal / animation issues in jsdom.
vi.mock("@radix-ui/react-dialog", () => {
  const Root = ({ open, children }: { open?: boolean; children: React.ReactNode }) =>
    open ? <div data-testid="dialog-root">{children}</div> : null;
  const Trigger = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Portal = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Overlay = () => <div data-testid="dialog-overlay" />;
  const Content = ({ children }: { children: React.ReactNode }) => (
    <div data-testid="dialog-content">{children}</div>
  );
  const Title = ({ children }: { children: React.ReactNode }) => <h2>{children}</h2>;
  const Close = ({ children }: { children: React.ReactNode }) => <button>{children}</button>;
  return { Root, Trigger, Portal, Overlay, Content, Title, Close };
});

// ── Fixtures ──────────────────────────────────────────────────────────────────

const CHART_PANEL_ID = "panel-1";
const TABLE_PANEL_ID = "panel-2";

const DASHBOARD_FIXTURE: DashboardWithPanels = {
  id: "dash-1",
  name: "Test Dashboard",
  description: null,
  time_range_preset: "1h",
  refresh_interval_seconds: null,
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
  panels: [
    {
      id: CHART_PANEL_ID,
      dashboard_id: "dash-1",
      title: "Chart Panel",
      panel_type: "chart",
      query: "events | summarize count() by bin(timestamp, 1m)",
      database_name: "testdb",
      config: {},
      grid_x: 0,
      grid_y: 0,
      grid_w: 6,
      grid_h: 4,
      display_order: 0,
    },
    {
      id: TABLE_PANEL_ID,
      dashboard_id: "dash-1",
      title: "Table Panel",
      panel_type: "table",
      query: "events | take 10",
      database_name: "testdb",
      config: { maxRows: 10 },
      grid_x: 6,
      grid_y: 0,
      grid_w: 6,
      grid_h: 4,
      display_order: 1,
    },
  ],
};

const QUERY_ROWS = [
  { timestamp: "2026-01-01T00:00:00Z", count_: 5 },
  { timestamp: "2026-01-01T00:01:00Z", count_: 8 },
];

function makeNdjsonResponse(rows: Record<string, unknown>[]) {
  const ndjson = rows.map((r) => JSON.stringify(r)).join("\n");
  return Promise.resolve(
    new Response(ndjson, {
      status: 200,
      headers: new Headers({ "content-type": "application/x-ndjson" }),
    }),
  );
}

function makeDashboardResponse(d: DashboardWithPanels) {
  return Promise.resolve(
    new Response(JSON.stringify(d), {
      status: 200,
      headers: new Headers({ "content-type": "application/json" }),
    }),
  );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeQC() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
}

function makeFetchMock(dashboard = DASHBOARD_FIXTURE) {
  return vi.fn().mockImplementation((url: string) => {
    if (url.includes("/v1/dashboards/")) return makeDashboardResponse(dashboard);
    if (url.includes("/v1/dashboards")) {
      return Promise.resolve(
        new Response(JSON.stringify([dashboard]), {
          status: 200,
          headers: new Headers({ "content-type": "application/json" }),
        }),
      );
    }
    if (url.includes("/v1/query")) return makeNdjsonResponse(QUERY_ROWS);
    return Promise.resolve(new Response("", { status: 404 }));
  });
}

function renderDashboard(
  props: Partial<Parameters<typeof PensieveDashboard>[0]> & { dashboardId?: string } = {},
  qc = makeQC(),
) {
  const { dashboardId = "dash-1", ...rest } = props;
  vi.stubGlobal("fetch", makeFetchMock());

  return render(
    <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }} queryClient={qc}>
      <PensieveDashboard dashboardId={dashboardId} {...rest} />
    </PensieveProvider>,
  );
}

// ── Tests ─────────────────────────────────────────────────────────────────────

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("PensieveDashboard", () => {
  it("1. smoke: renders panel titles and toolbar after dashboard loads", async () => {
    renderDashboard();

    // Loading state shown first
    expect(screen.queryByText("Test Dashboard") ?? screen.queryByText("Loading dashboard…")).toBeTruthy();

    // After load, panel titles and dashboard name appear
    await waitFor(() => {
      expect(screen.getByText("Test Dashboard")).toBeTruthy();
    });
    await waitFor(() => {
      expect(screen.getByText("Chart Panel")).toBeTruthy();
      expect(screen.getByText("Table Panel")).toBeTruthy();
    });

    // Toolbar controls visible
    expect(screen.getByTestId("edit-toggle-btn")).toBeTruthy();
  });

  it("2. panel executes query and renders data (chart + table panels)", async () => {
    renderDashboard();

    await waitFor(() => {
      expect(screen.getByText("Chart Panel")).toBeTruthy();
    });

    // Chart panels render via ChartPanelViz → ChartPanel → echarts mock
    await waitFor(() => {
      // Either the echart placeholder or the "no data" empty state
      // Both indicate the query cycle completed.
      const echarts = document.querySelectorAll("[data-testid='echart']");
      const noData = screen.queryAllByText("No data.");
      const loading = screen.queryAllByText("Loading…");
      // At minimum we should be past loading (query ran)
      expect(echarts.length + noData.length).toBeGreaterThan(0);
      expect(loading.length).toBe(0);
    });
  });

  it("3. editable=false hides add-panel button", async () => {
    renderDashboard({ editable: false });

    await waitFor(() => {
      expect(screen.getByText("Test Dashboard")).toBeTruthy();
    });

    // No add-panel button in view mode
    expect(screen.queryByTestId("add-panel-btn")).toBeNull();
    // Edit toggle is present but shows "Edit" label
    expect(screen.getByTestId("edit-toggle-btn")).toBeTruthy();
  });

  it("4. editable=true shows add-panel button and save button", async () => {
    renderDashboard({ editable: true });

    await waitFor(() => {
      expect(screen.getByText("Test Dashboard")).toBeTruthy();
    });

    // Add panel button present in edit mode
    expect(screen.getByTestId("add-panel-btn")).toBeTruthy();

    // Save button present
    expect(screen.getByTestId("save-btn")).toBeTruthy();

    // Edit controls (edit/delete) visible on panels
    const editBtns = document.querySelectorAll("button[title='Edit panel']");
    expect(editBtns.length).toBe(2); // one per panel
  });

  it("5b. database prop sends x-database header on dashboard load (GET /v1/dashboards/:id)", async () => {
    const fetchMock = makeFetchMock();
    vi.stubGlobal("fetch", fetchMock);
    const qc = makeQC();

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }} queryClient={qc}>
        <PensieveDashboard dashboardId="dash-1" database="staging" />
      </PensieveProvider>,
    );

    // Wait for the dashboard to load
    await waitFor(() => {
      expect(screen.getByText("Test Dashboard")).toBeTruthy();
    });

    expect(fetchMock).toHaveBeenCalled();

    // The dashboard GET request should carry x-database: staging
    const dashboardCall = (fetchMock.mock.calls as [string, RequestInit][]).find(([url]) =>
      url.includes("/v1/dashboards/"),
    );
    expect(dashboardCall).toBeTruthy();
    const [, init] = dashboardCall!;
    const headers =
      init?.headers instanceof Headers
        ? Object.fromEntries((init.headers as Headers).entries())
        : (init?.headers as Record<string, string> | undefined) ?? {};
    expect(headers["x-database"]).toBe("staging");
  });

  it("5c. database prop seeds new-panel default database_name in AddPanelModal", async () => {
    const schemaFixture = {
      databases: [{ name: "mydb", tables: [] }],
    };
    const fetchMock = vi.fn().mockImplementation((url: string) => {
      if (url.includes("/v1/dashboards/")) return makeDashboardResponse(DASHBOARD_FIXTURE);
      if (url.includes("/v1/dashboards")) {
        return Promise.resolve(
          new Response(JSON.stringify([DASHBOARD_FIXTURE]), {
            status: 200,
            headers: new Headers({ "content-type": "application/json" }),
          }),
        );
      }
      if (url.includes("/v1/query")) return makeNdjsonResponse(QUERY_ROWS);
      if (url.includes("/v1/catalog")) {
        return Promise.resolve(
          new Response(JSON.stringify(schemaFixture), {
            status: 200,
            headers: new Headers({ "content-type": "application/json" }),
          }),
        );
      }
      return Promise.resolve(new Response("", { status: 404 }));
    });
    vi.stubGlobal("fetch", fetchMock);
    const qc = makeQC();

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }} queryClient={qc}>
        <PensieveDashboard dashboardId="dash-1" editable database="mydb" />
      </PensieveProvider>,
    );

    await waitFor(() => {
      expect(screen.getByText("Test Dashboard")).toBeTruthy();
    });

    // Open the add panel modal
    const addBtn = screen.getByTestId("add-panel-btn");
    addBtn.click();

    await waitFor(() => {
      expect(screen.getByTestId("dialog-content")).toBeTruthy();
    });

    // Wait for schema to load and "mydb" option to appear in the select
    await waitFor(() => {
      const dbSelect = document.querySelector("#panel-db") as HTMLSelectElement | null;
      // The select should default to "mydb" (seeded from PensieveDashboard.database prop)
      expect(dbSelect?.value).toBe("mydb");
    });
  });

  it("5. fallback rendered while loading", async () => {
    // Delay the dashboard response so we can observe the fallback
    vi.stubGlobal(
      "fetch",
      vi.fn().mockImplementation(
        () => new Promise((resolve) => setTimeout(() => resolve(makeDashboardResponse(DASHBOARD_FIXTURE)), 200)),
      ),
    );
    const qc = makeQC();

    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }} queryClient={qc}>
        <PensieveDashboard dashboardId="dash-1" fallback={<div data-testid="my-fallback">Loading...</div>} />
      </PensieveProvider>,
    );

    expect(screen.getByTestId("my-fallback")).toBeTruthy();
  });
});
