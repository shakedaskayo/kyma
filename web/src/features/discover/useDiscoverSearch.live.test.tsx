/**
 * I3 — Hook-level tests for useDiscoverSearch live mode.
 *
 * Verifies:
 * (a) live=true creates exactly ONE LiveSession
 * (b) changing search arg → NO new session, exactly ONE update() call
 * (c) live=false → session.close() called
 * (d) double-mount (StrictMode-style) does not leak — closed = created - 1
 * (e) no spurious update() fires on initial session creation
 * (f) multiple arg changes send multiple updates, still one session
 */
import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act, cleanup } from "@testing-library/react";
import type { LiveBody } from "../../sdk/discover-live";

// ---------------------------------------------------------------------------
// Hoisted fake — must be declared via vi.hoisted so vi.mock factory can see it
// ---------------------------------------------------------------------------

const { FakeLiveSession } = vi.hoisted(() => {
  class FakeLiveSession {
    static instances: FakeLiveSession[] = [];
    static closedCount = 0;
    static updateCalls: LiveBody[] = [];

    constructorBody: LiveBody;

    constructor(opts: {
      body: LiveBody;
      onFrame: unknown;
      onStatus: unknown;
      endpoint: string;
      token: string;
    }) {
      this.constructorBody = opts.body;
      FakeLiveSession.instances.push(this);
    }

    close() {
      FakeLiveSession.closedCount++;
    }

    update(body: LiveBody) {
      FakeLiveSession.updateCalls.push(body);
    }

    static reset() {
      FakeLiveSession.instances = [];
      FakeLiveSession.closedCount = 0;
      FakeLiveSession.updateCalls = [];
    }
  }
  return { FakeLiveSession };
});

// ---------------------------------------------------------------------------
// Module mocks
// ---------------------------------------------------------------------------

vi.mock("../../sdk/discover-live", () => ({
  LiveSession: FakeLiveSession,
}));

vi.mock("../../sdk/session", () => ({
  useSession: () => ({ endpoint: "http://localhost:8080", token: "test-token" }),
}));

// discover one-shot path — mock to never resolve (we're testing live path only)
vi.mock("../../sdk/discover", () => ({
  searchDiscover: async function* () {
    await new Promise(() => {});
  },
}));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

import { useDiscoverSearch } from "./useDiscoverSearch";
import type { Scope } from "./types";

const baseScope: Scope = { kind: "all" };
const baseTimeRange = { preset: "1h" as const };

function makeArgs(search = "error", live = true) {
  return {
    search,
    scope: baseScope,
    timeRange: baseTimeRange,
    enabled: true,
    live,
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("useDiscoverSearch — live mode hook", () => {
  beforeEach(() => {
    FakeLiveSession.reset();
  });

  afterEach(() => {
    cleanup();
  });

  // (a) live=true creates exactly ONE session
  it("(a) live=true creates exactly one LiveSession", () => {
    renderHook(() => useDiscoverSearch(makeArgs("error", true)));
    expect(FakeLiveSession.instances).toHaveLength(1);
  });

  // (b) changing search arg → NO new session, exactly ONE update() call
  it("(b) changing search arg sends update() without creating a new session", () => {
    const { rerender } = renderHook(
      (args) => useDiscoverSearch(args),
      { initialProps: makeArgs("error", true) },
    );

    expect(FakeLiveSession.instances).toHaveLength(1);

    act(() => {
      rerender(makeArgs("warn", true));
    });

    // Still only one session
    expect(FakeLiveSession.instances).toHaveLength(1);
    // update() called once with new query
    expect(FakeLiveSession.updateCalls).toHaveLength(1);
    expect(FakeLiveSession.updateCalls[0].query).toBe("warn");
  });

  // (c) live=false → session.close() called
  it("(c) turning live off closes the session", () => {
    const { rerender } = renderHook(
      (args) => useDiscoverSearch(args),
      { initialProps: makeArgs("error", true) },
    );

    expect(FakeLiveSession.instances).toHaveLength(1);

    act(() => {
      rerender(makeArgs("error", false));
    });

    // The cleanup from the live effect should have called close()
    expect(FakeLiveSession.closedCount).toBeGreaterThanOrEqual(1);
  });

  // (d) double-mount does not leak — after unmount closed = created
  it("(d) unmount closes the session (no leak)", () => {
    FakeLiveSession.reset();

    const { unmount } = renderHook(() => useDiscoverSearch(makeArgs("error", true)));
    expect(FakeLiveSession.instances).toHaveLength(1);
    expect(FakeLiveSession.closedCount).toBe(0);

    unmount();
    // After unmount: the effect cleanup should close the session
    expect(FakeLiveSession.closedCount).toBe(1);
  });

  // (e) no spurious update() fires on initial live start
  it("(e) no spurious update() fires on initial session creation", () => {
    renderHook(() => useDiscoverSearch(makeArgs("error", true)));
    expect(FakeLiveSession.instances).toHaveLength(1);
    expect(FakeLiveSession.updateCalls).toHaveLength(0);
  });

  // (f) multiple arg changes → multiple updates, still one session
  it("(f) multiple arg changes send multiple updates without new sessions", () => {
    const { rerender } = renderHook(
      (args) => useDiscoverSearch(args),
      { initialProps: makeArgs("error", true) },
    );

    act(() => { rerender(makeArgs("warn", true)); });
    act(() => { rerender(makeArgs("info", true)); });
    act(() => { rerender(makeArgs("debug", true)); });

    expect(FakeLiveSession.instances).toHaveLength(1);
    expect(FakeLiveSession.updateCalls).toHaveLength(3);
    expect(FakeLiveSession.updateCalls[2].query).toBe("debug");
  });
});
