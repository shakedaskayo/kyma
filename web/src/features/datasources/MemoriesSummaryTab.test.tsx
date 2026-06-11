import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { MemorySourceSummary } from "@/sdk/memory";

const h = vi.hoisted(() => ({
  rows: [] as MemorySourceSummary[],
  isLoading: false,
  error: null as Error | null,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));

vi.mock("./useDataSources", () => ({
  useMemorySourceSummary: () => ({ data: h.rows, isLoading: h.isLoading, error: h.error }),
}));

import { groupBySource, MemoriesSummaryTab } from "./MemoriesSummaryTab";

afterEach(() => {
  cleanup();
  h.rows = [];
  h.isLoading = false;
  h.error = null;
});

describe("MemoriesSummaryTab", () => {
  it("shows the empty state when there are no memories", () => {
    render(<MemoriesSummaryTab />);
    expect(screen.getByText("No memories yet")).toBeTruthy();
  });

  it("renders one card per source, sorted by total desc", () => {
    h.rows = [
      { source: "manual", realm: "default", count: 2 },
      { source: "claude-code", realm: "proj-a", count: 10 },
      { source: "claude-code", realm: "proj-b", count: 5 },
    ];
    render(<MemoriesSummaryTab />);
    expect(screen.getByText("claude-code")).toBeTruthy();
    expect(screen.getByText("manual")).toBeTruthy();
    expect(screen.getByText("15 memories")).toBeTruthy();
    expect(screen.getByText("proj-a")).toBeTruthy();
    // Header prose links to the full store.
    expect(screen.getByText(/The full store lives in/)).toBeTruthy();
  });
});

describe("groupBySource", () => {
  it("groups, sums, and sorts sources by total desc and realms by count desc", () => {
    const groups = groupBySource([
      { source: "manual", realm: "default", count: 2 },
      { source: "claude-code", realm: "proj-b", count: 5 },
      { source: "claude-code", realm: "proj-a", count: 10 },
    ]);
    expect(groups.map((g) => g.source)).toEqual(["claude-code", "manual"]);
    expect(groups[0].total).toBe(15);
    expect(groups[0].realms.map((r) => r.realm)).toEqual(["proj-a", "proj-b"]);
  });
});
