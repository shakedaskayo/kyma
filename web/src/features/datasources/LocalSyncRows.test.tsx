import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { DataSourceWatcher, WatcherSettings } from "@/sdk/datasources";

const h = vi.hoisted(() => ({
  watchers: [] as DataSourceWatcher[],
  settings: { cc_sync_enabled: true } as WatcherSettings,
  isLoading: false,
  error: null as Error | null,
  updateSettings: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));

vi.mock("./useDataSources", () => ({
  useDataSourceWatchers: () => ({ data: h.watchers, isLoading: h.isLoading, error: h.error }),
  useWatcherSettings: () => ({ data: h.settings }),
  useUpdateWatcherSettings: () => ({ mutate: h.updateSettings, isPending: false }),
}));

// BrandIcon pulls the theme store; stub it to keep the test DOM-only.
vi.mock("./BrandIcon", () => ({
  BrandIcon: ({ brand }: { brand: string }) => <span data-testid={`brand-${brand}`} />,
}));

import { CcSyncRow, FiledropRow } from "./LocalSyncRows";

function ccWatcher(over: Partial<DataSourceWatcher> = {}): DataSourceWatcher {
  return {
    id: "cc-sync",
    kind: "cc_sync",
    node_host: "mbp.local",
    node_id: "n1",
    identity: "shaked",
    config: { root: "~/.claude/projects", poll_secs: 30 },
    started_at: new Date().toISOString(),
    last_heartbeat_at: new Date().toISOString(),
    last_scan: {
      seen: 4,
      processed: 4,
      errors: 0,
      duration_ms: 88,
      at: new Date().toISOString(),
      detail: {
        realms: [
          { realm: "proj-a", upserted: 3, skipped: 1, user_edited: 0, edges_added: 2, archived: 0 },
        ],
      },
    },
    stale: false,
    ...over,
  };
}

afterEach(() => {
  cleanup();
  h.watchers = [];
  h.settings = { cc_sync_enabled: true };
  h.isLoading = false;
  h.error = null;
  h.updateSettings.mockReset();
});

describe("CcSyncRow", () => {
  it("renders the pre-installed row with the claude brand and live state", () => {
    h.watchers = [ccWatcher()];
    render(<CcSyncRow />);
    expect(screen.getByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("Pre-installed")).toBeTruthy();
    expect(screen.getByTestId("brand-claude")).toBeTruthy();
    expect(screen.getByText("Live")).toBeTruthy();
    expect(screen.getByText("mbp.local")).toBeTruthy();
  });

  it("expands to the per-realm table", () => {
    h.watchers = [ccWatcher()];
    render(<CcSyncRow />);
    fireEvent.click(screen.getByRole("button", { name: /details/i }));
    expect(screen.getByText("proj-a")).toBeTruthy();
    expect(screen.getByText("Upserted")).toBeTruthy();
    expect(screen.getByText("User edited")).toBeTruthy();
  });

  it("pauses sync from the row", () => {
    h.watchers = [ccWatcher()];
    render(<CcSyncRow />);
    fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    expect(h.updateSettings).toHaveBeenCalledWith({ cc_sync_enabled: false });
  });

  it("shows paused state with an Enable action when sync is off", () => {
    h.watchers = [];
    h.settings = { cc_sync_enabled: false };
    render(<CcSyncRow />);
    expect(screen.getByText(/paused/i)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Enable" }));
    expect(h.updateSettings).toHaveBeenCalledWith({ cc_sync_enabled: true });
  });

  it("shows starting state when enabled but no watcher reported yet", () => {
    h.watchers = [];
    h.settings = { cc_sync_enabled: true };
    render(<CcSyncRow />);
    expect(screen.getByText(/starting/i)).toBeTruthy();
  });
});

describe("FiledropRow", () => {
  it("renders the watched prefixes and liveness", () => {
    const w = ccWatcher({
      id: "fd1",
      kind: "filedrop",
      config: { prefixes: ["/drop/a", "/drop/b"], poll_secs: 10 },
      last_scan: null,
    });
    render(<FiledropRow watcher={w} />);
    expect(screen.getByText("File drop")).toBeTruthy();
    expect(screen.getByText("/drop/a, /drop/b")).toBeTruthy();
    expect(screen.getByText("Live")).toBeTruthy();
  });
});
