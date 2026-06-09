import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { ReviewItem } from "@/sdk/review";

// Mutable holder so each test can set what the inbox "loads".
const h = vi.hoisted(() => ({
  items: [] as ReviewItem[],
  isLoading: false,
  error: null as Error | null,
}));

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => ({}),
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));

vi.mock("./useReview", () => ({
  useReviewItems: () => ({ data: h.items, isLoading: h.isLoading, error: h.error }),
  useReviewCount: () => ({ data: { pending: 0, post_hoc: 0 } }),
  useResolveReview: () => ({ mutate: vi.fn(), isPending: false }),
  useBulkReview: () => ({ mutate: vi.fn(), isPending: false }),
}));

import { ReviewInbox } from "./ReviewInbox";

function item(over: Partial<ReviewItem> = {}): ReviewItem {
  return {
    id: "1",
    realm: "r",
    operation: "invalidate",
    mode: "gate",
    status: "pending",
    confidence: 0.5,
    reason: "contradiction",
    source: "dreaming",
    source_run_id: null,
    payload: { op: "invalidate", target_id: "memory:t", replacement: { content: "new fact" } },
    inverse: null,
    created_at: new Date().toISOString(),
    resolved_at: null,
    resolved_by: null,
    resolution_comment: null,
    ...over,
  };
}

afterEach(() => {
  cleanup();
  h.items = [];
  h.isLoading = false;
  h.error = null;
});

describe("ReviewInbox", () => {
  it("shows the empty state when there are no candidates", () => {
    h.items = [];
    render(<ReviewInbox />);
    expect(screen.getByText("Nothing to review")).toBeTruthy();
  });

  it("renders a card per queue item", () => {
    h.items = [item(), item({ id: "2", operation: "merge" })];
    render(<ReviewInbox />);
    expect(screen.getByText("Invalidate")).toBeTruthy();
    expect(screen.getByText("Merge")).toBeTruthy();
    expect(screen.getAllByTestId("candidate-card")).toHaveLength(2);
  });

  it("renders the status filter chips", () => {
    render(<ReviewInbox />);
    expect(screen.getByText("Pending")).toBeTruthy();
    expect(screen.getByText("All")).toBeTruthy();
  });
});
