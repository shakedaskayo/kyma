import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { DataSourceWatcher } from "@/sdk/datasources";

const h = vi.hoisted(() => ({
  watchers: [] as DataSourceWatcher[],
  isLoading: false,
  error: null as Error | null,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));

vi.mock("./useDataSources", () => ({
  useDataSourceWatchers: () => ({ data: h.watchers, isLoading: h.isLoading, error: h.error }),
}));

import { SyncTab } from "./SyncTab";

function ccWatcher(over: Partial<DataSourceWatcher> = {}): DataSourceWatcher {
  return {
    id: "cc1",
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
  h.isLoading = false;
  h.error = null;
});

describe("SyncTab", () => {
  it("shows the KYMA_CC_WATCH empty state when no cc_sync watcher runs", () => {
    h.watchers = [{ ...ccWatcher(), kind: "filedrop" }];
    render(<SyncTab />);
    expect(screen.getByText("Claude Code sync is not running")).toBeTruthy();
    expect(screen.getByText("KYMA_CC_WATCH=1")).toBeTruthy();
  });

  it("renders the summary line and per-realm table when running", () => {
    h.watchers = [ccWatcher()];
    render(<SyncTab />);
    expect(screen.getByText("~/.claude/projects")).toBeTruthy();
    expect(screen.getByText("mbp.local")).toBeTruthy();
    expect(screen.getByText("Live")).toBeTruthy();
    expect(screen.getByText("proj-a")).toBeTruthy();
    expect(screen.getByText("Upserted")).toBeTruthy();
    expect(screen.getByText("User edited")).toBeTruthy();
    // Link to /memory in the realms note.
    expect(screen.getByText("Memory")).toBeTruthy();
  });

  it("shows 'No realms synced yet' for a fresh watcher", () => {
    h.watchers = [ccWatcher({ last_scan: null })];
    render(<SyncTab />);
    expect(screen.getByText("No realms synced yet.")).toBeTruthy();
  });
});
