import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { DataSourceWatcher, WatcherSettings } from "@/sdk/datasources";

// Mutable holder so each test can set what the tab "loads".
const h = vi.hoisted(() => ({
  watchers: [] as DataSourceWatcher[],
  settings: { cc_sync_enabled: true } as WatcherSettings,
  isLoading: false,
  error: null as Error | null,
}));

vi.mock("./useDataSources", () => ({
  useDataSourceWatchers: () => ({ data: h.watchers, isLoading: h.isLoading, error: h.error }),
  useWatcherSettings: () => ({ data: h.settings }),
  useUpdateWatcherSettings: () => ({ mutate: vi.fn(), isPending: false }),
}));

import { WatchersTab } from "./WatchersTab";

function watcher(over: Partial<DataSourceWatcher> = {}): DataSourceWatcher {
  return {
    id: "w1",
    kind: "filedrop",
    node_host: "node-a",
    node_id: "n1",
    identity: "kyma",
    config: { prefixes: ["ingest", "drop"], poll_secs: 30 },
    started_at: new Date().toISOString(),
    last_heartbeat_at: new Date().toISOString(),
    last_scan: {
      seen: 10,
      processed: 9,
      errors: 1,
      duration_ms: 420,
      at: new Date().toISOString(),
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
});

describe("WatchersTab", () => {
  it("shows the empty state with file-drop hint when nothing is running (cc enabled)", () => {
    h.settings = { cc_sync_enabled: true };
    render(<WatchersTab />);
    expect(screen.getByText("No file watchers running")).toBeTruthy();
    expect(screen.getByText("KYMA_FILEDROP_ENABLED=1")).toBeTruthy();
    // cc_sync already enabled — no enable button shown
    expect(screen.queryByRole("button", { name: /Enable Claude Code sync/ })).toBeNull();
  });

  it("shows Enable button when cc_sync is disabled", () => {
    h.settings = { cc_sync_enabled: false };
    render(<WatchersTab />);
    expect(screen.getByText("No file watchers running")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Enable Claude Code sync" })).toBeTruthy();
  });

  it("renders a card per watcher with kind, target, and scan line", () => {
    h.watchers = [
      watcher(),
      watcher({
        id: "w2",
        kind: "cc_sync",
        config: { root: "~/.claude/projects", poll_secs: 60 },
        stale: true,
        last_scan: null,
      }),
    ];
    render(<WatchersTab />);
    expect(screen.getByText("File drop")).toBeTruthy();
    expect(screen.getByText("Claude Code sync")).toBeTruthy();
    expect(screen.getByText("ingest, drop")).toBeTruthy();
    expect(screen.getByText("~/.claude/projects")).toBeTruthy();
    expect(screen.getByText(/9\/10 files, 1 error/)).toBeTruthy();
    expect(screen.getByText("No scans yet")).toBeTruthy();
    expect(screen.getByText("Live")).toBeTruthy();
    expect(screen.getByText("Stale")).toBeTruthy();
  });

  it("shows the error state", () => {
    h.error = new Error("boom");
    render(<WatchersTab />);
    expect(screen.getByText(/Failed to load watchers: boom/)).toBeTruthy();
  });
});
