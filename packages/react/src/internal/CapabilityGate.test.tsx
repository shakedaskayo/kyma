import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, cleanup, waitFor } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import { KymaProvider } from "../provider/KymaProvider";
import { CapabilityGate } from "./CapabilityGate";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

/** Wraps CapabilityGate with a no-retry QueryClient for deterministic tests. */
function CapWrapper({
  capability,
  qc,
  fallback,
}: {
  capability: string;
  qc: QueryClient;
  fallback?: React.ReactNode;
}) {
  return (
    <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }} queryClient={qc}>
      <CapabilityGate capability={capability} fallback={fallback}>
        <div data-testid="content">content</div>
      </CapabilityGate>
    </KymaProvider>
  );
}

const BASE_CAPS = {
  mode: "server",
  data_sources: true,
  credentials: true,
  oauth: true,
  saved_views: true,
  users_admin: true,
  explore_live: true,
};

describe("CapabilityGate", () => {
  it("renders children when capability is present (fail-open on load)", () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        json: async () => BASE_CAPS,
      }),
    );
    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }} queryClient={qc}>
        <CapabilityGate capability="data_sources">
          <div data-testid="content">content</div>
        </CapabilityGate>
      </KymaProvider>,
    );
    // While loading (or on success), children are rendered (fail-open)
    expect(screen.getByTestId("content")).toBeTruthy();
  });

  it("renders children while capabilities are loading (fail-open)", () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    // Never resolves — simulates slow/hung capabilities fetch
    vi.stubGlobal("fetch", vi.fn().mockReturnValue(new Promise(() => {})));
    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }} queryClient={qc}>
        <CapabilityGate capability="data_sources">
          <div data-testid="content">content</div>
        </CapabilityGate>
      </KymaProvider>,
    );
    // Should render children even while loading — fail-open for availability
    expect(screen.getByTestId("content")).toBeTruthy();
  });

  it("renders 'not available' fallback when capability is absent", async () => {
    const capsWithout = { ...BASE_CAPS, data_sources: false };
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        // transport checks res.headers.get("content-type") — must be present
        headers: new Headers({ "content-type": "application/json" }),
        json: async () => capsWithout,
      }),
    );
    render(<CapWrapper capability="data_sources" qc={qc} />);
    // After capabilities load, the capability is absent — show default fallback
    await waitFor(() => {
      expect(screen.queryByTestId("capability-unavailable")).toBeTruthy();
    }, { timeout: 3000 });
    expect(screen.queryByTestId("content")).toBeFalsy();
  });

  it("renders custom fallback prop when capability is absent", async () => {
    const capsWithout = { ...BASE_CAPS, data_sources: false };
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: 0 } } });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        headers: new Headers({ "content-type": "application/json" }),
        json: async () => capsWithout,
      }),
    );
    render(
      <CapWrapper
        capability="data_sources"
        qc={qc}
        fallback={<div data-testid="custom-fallback">not supported</div>}
      />,
    );
    await waitFor(() => {
      expect(screen.queryByTestId("custom-fallback")).toBeTruthy();
    }, { timeout: 3000 });
    expect(screen.queryByTestId("content")).toBeFalsy();
  });
});
