import { afterEach, describe, expect, it } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import { KymaProvider } from "./KymaProvider";
import { useKymaClient } from "./context";

function Probe() {
  const client = useKymaClient();
  return <div data-testid="ep">{client.transport.endpoint}</div>;
}

afterEach(() => cleanup());

describe("KymaProvider", () => {
  it("provides a client and renders a kyma-root with theme vars", () => {
    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }}>
        <Probe />
      </KymaProvider>,
    );
    expect(screen.getByTestId("ep").textContent).toBe("https://kyma.test");
    const root = document.querySelector(".kyma-root") as HTMLElement;
    expect(root).toBeTruthy();
    expect(root.style.getPropertyValue("--kyma-background")).not.toBe("");
  });

  it("throws a descriptive error when hooks are used outside the provider", () => {
    // silence React error boundary console noise
    const orig = console.error;
    console.error = () => {};
    expect(() => render(<Probe />)).toThrow(/KymaProvider/);
    console.error = orig;
  });

  it("inherit mode sets no inline --kyma-background var", () => {
    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }} theme="inherit">
        <Probe />
      </KymaProvider>,
    );
    // In inherit mode no inline vars should be set on the root
    const roots = document.querySelectorAll(".kyma-root");
    // Check the first root element (the outer .kyma-root div)
    const outerRoot = roots[0] as HTMLElement;
    expect(outerRoot.style.getPropertyValue("--kyma-background")).toBe("");
  });

  it("custom partial theme overrides one var", () => {
    render(
      <KymaProvider
        endpoint="https://kyma.test"
        auth={{ token: "t" }}
        theme={{ background: "0 0% 50%" }}
      >
        <Probe />
      </KymaProvider>,
    );
    const root = document.querySelector(".kyma-root") as HTMLElement;
    expect(root.style.getPropertyValue("--kyma-background")).toBe("0 0% 50%");
  });

  it("host queryClient is used when provided", () => {
    const hostQC = new QueryClient();
    // Should not throw and client should be accessible
    render(
      <KymaProvider endpoint="https://kyma.test" auth={{ token: "t" }} queryClient={hostQC}>
        <Probe />
      </KymaProvider>,
    );
    expect(screen.getByTestId("ep").textContent).toBe("https://kyma.test");
  });
});
