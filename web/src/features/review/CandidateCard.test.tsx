import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { CandidateCard } from "./CandidateCard";
import type { ReviewItem } from "@/sdk/review";

// The card renders a router <Link> for the run deep-link; stub it to a plain <a>.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}));

afterEach(cleanup);

function item(over: Partial<ReviewItem> = {}): ReviewItem {
  return {
    id: "1",
    realm: "r",
    operation: "merge",
    mode: "gate",
    status: "pending",
    confidence: 0.42,
    reason: "near-duplicate",
    source: "dreaming",
    source_run_id: null,
    payload: { op: "merge", into_id: "memory:a", from_ids: ["memory:b"] },
    inverse: null,
    created_at: new Date().toISOString(),
    resolved_at: null,
    resolved_by: null,
    resolution_comment: null,
    ...over,
  };
}

describe("CandidateCard", () => {
  it("renders op label, confidence, and reason for a gated merge", () => {
    render(<CandidateCard item={item()} onAction={() => {}} />);
    expect(screen.getByText("Merge")).toBeTruthy();
    expect(screen.getByText(/conf 0\.42/)).toBeTruthy();
    expect(screen.getByText("near-duplicate")).toBeTruthy();
  });

  it("calls onAction('approve') when Approve is clicked (no edit)", () => {
    const onAction = vi.fn();
    render(<CandidateCard item={item()} onAction={onAction} />);
    fireEvent.click(screen.getByText("Approve"));
    expect(onAction).toHaveBeenCalledWith("approve", undefined);
  });

  it("calls onAction('reject') when Reject is clicked", () => {
    const onAction = vi.fn();
    render(<CandidateCard item={item()} onAction={onAction} />);
    fireEvent.click(screen.getByText("Reject"));
    expect(onAction).toHaveBeenCalledWith("reject");
  });

  it("shows Undo (not Approve) for an auto-applied post-hoc row", () => {
    const onAction = vi.fn();
    render(
      <CandidateCard
        item={item({ status: "auto_applied", mode: "post_hoc", operation: "update" })}
        onAction={onAction}
      />,
    );
    expect(screen.queryByText("Approve")).toBeNull();
    fireEvent.click(screen.getByText("Undo"));
    expect(onAction).toHaveBeenCalledWith("undo");
  });

  it("edit-then-approve sends the edited payload for an update op", () => {
    const onAction = vi.fn();
    const it_ = item({
      operation: "update",
      payload: { op: "update", target_id: "memory:t", new_content: "old text" },
    });
    render(<CandidateCard item={it_} onAction={onAction} />);
    fireEvent.click(screen.getByText("Edit"));
    const ta = screen.getByRole("textbox") as HTMLTextAreaElement;
    fireEvent.change(ta, { target: { value: "fixed text" } });
    fireEvent.click(screen.getByText("Save & approve"));
    expect(onAction).toHaveBeenCalledTimes(1);
    const [action, payload] = onAction.mock.calls[0];
    expect(action).toBe("approve");
    expect((payload as { new_content: string }).new_content).toBe("fixed text");
  });
});
