/**
 * Integration test for StreamView's freeze-on-scroll logic.
 *
 * Verifies that scrolling the StreamView container down freezes rendering
 * and causes the "N new events ↑" pill to appear when more rows arrive.
 */
import { afterEach, describe, expect, it } from "vitest";
import { render, act, cleanup } from "@testing-library/react";
import { StreamView } from "./StreamView";
import type { StreamRow } from "./stream";
import type { SourceKey, SourceState } from "./types";

afterEach(() => {
  cleanup();
});

function makeRows(n: number, source: SourceKey = "db.logs"): StreamRow[] {
  return Array.from({ length: n }, (_, i) => ({
    ts: Date.now() - i * 1000,
    source,
    row: { msg: `event-${i}` },
  }));
}

function makeSourcesMap(source: SourceKey = "db.logs"): Map<SourceKey, SourceState> {
  const state: SourceState = {
    source,
    rows: [{ msg: "event-0" }],
    timestampColumn: null,
    hasTimestamp: false,
    progress: "done",
    total: 1,
    capped: false,
    droppedClauses: [],
  };
  return new Map<SourceKey, SourceState>([[source, state]]);
}

describe("StreamView freeze-on-scroll integration", () => {
  it("shows the 'new events' pill after scrolling down and receiving more rows", () => {
    const initialRows = makeRows(5);
    const sources = makeSourcesMap();

    const { queryByRole, rerender, container } = render(
      <StreamView
        rows={initialRows}
        sources={sources}
        columns={[]}
        onOpenRow={() => {}}
      />,
    );

    // Initially no pill (there's a "show N more" button but no "new events" pill)
    expect(queryByRole("button", { name: /new event/i })).toBeNull();

    // Grab the scroll container (the root div with ref={scrollRef})
    const scrollDiv = container.firstElementChild as HTMLDivElement;
    expect(scrollDiv).not.toBeNull();

    // Simulate the user scrolling down: set scrollTop > 0 and dispatch scroll
    act(() => {
      Object.defineProperty(scrollDiv, "scrollTop", { value: 100, writable: true, configurable: true });
      scrollDiv.dispatchEvent(new Event("scroll", { bubbles: true }));
    });

    // After scrolling down, re-render with MORE rows
    const moreRows = makeRows(8);
    rerender(
      <StreamView
        rows={moreRows}
        sources={sources}
        columns={[]}
        onOpenRow={() => {}}
      />,
    );

    // The pill should now appear announcing the 3 new events (8 - 5)
    const pill = queryByRole("button", { name: /new event/i });
    expect(pill).not.toBeNull();
    expect(pill!.textContent).toMatch(/3 new event/);
  });

  it("hides the pill when jumpToTop is clicked", () => {
    const initialRows = makeRows(5);
    const sources = makeSourcesMap();

    const { queryByRole, rerender, container } = render(
      <StreamView
        rows={initialRows}
        sources={sources}
        columns={[]}
        onOpenRow={() => {}}
      />,
    );

    const scrollDiv = container.firstElementChild as HTMLDivElement;

    // Scroll down
    act(() => {
      Object.defineProperty(scrollDiv, "scrollTop", { value: 50, writable: true, configurable: true });
      scrollDiv.dispatchEvent(new Event("scroll", { bubbles: true }));
    });

    // More rows arrive
    const moreRows = makeRows(7);
    rerender(
      <StreamView
        rows={moreRows}
        sources={sources}
        columns={[]}
        onOpenRow={() => {}}
      />,
    );

    const pill = queryByRole("button", { name: /new event/i });
    expect(pill).not.toBeNull();

    // Click the pill — should jump to top and unfreeze
    act(() => {
      // Simulate scrollTop going back to 0 via jumpToTop
      Object.defineProperty(scrollDiv, "scrollTop", { value: 0, writable: true, configurable: true });
      pill!.click();
    });

    // After jumping to top, pill should disappear
    expect(queryByRole("button", { name: /new event/i })).toBeNull();
  });
});
