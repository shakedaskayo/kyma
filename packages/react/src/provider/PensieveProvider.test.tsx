import { afterEach, describe, expect, it } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import { QueryClient } from "@tanstack/react-query";
import { PensieveProvider } from "./PensieveProvider";
import { usePensieveClient } from "./context";

function Probe() {
  const client = usePensieveClient();
  return <div data-testid="ep">{client.transport.endpoint}</div>;
}

afterEach(() => cleanup());

describe("PensieveProvider", () => {
  it("provides a client and renders a pensieve-root with theme vars", () => {
    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }}>
        <Probe />
      </PensieveProvider>,
    );
    expect(screen.getByTestId("ep").textContent).toBe("https://pensieve.test");
    const root = document.querySelector(".pensieve-root") as HTMLElement;
    expect(root).toBeTruthy();
    expect(root.style.getPropertyValue("--pensieve-background")).not.toBe("");
  });

  it("throws a descriptive error when hooks are used outside the provider", () => {
    // silence React error boundary console noise
    const orig = console.error;
    console.error = () => {};
    expect(() => render(<Probe />)).toThrow(/PensieveProvider/);
    console.error = orig;
  });

  it("inherit mode sets no inline --pensieve-background var", () => {
    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} theme="inherit">
        <Probe />
      </PensieveProvider>,
    );
    // In inherit mode no inline vars should be set on the root
    const roots = document.querySelectorAll(".pensieve-root");
    // Check the first root element (the outer .pensieve-root div)
    const outerRoot = roots[0] as HTMLElement;
    expect(outerRoot.style.getPropertyValue("--pensieve-background")).toBe("");
  });

  it("custom partial theme overrides one var", () => {
    render(
      <PensieveProvider
        endpoint="https://pensieve.test"
        auth={{ token: "t" }}
        theme={{ background: "0 0% 50%" }}
      >
        <Probe />
      </PensieveProvider>,
    );
    const root = document.querySelector(".pensieve-root") as HTMLElement;
    expect(root.style.getPropertyValue("--pensieve-background")).toBe("0 0% 50%");
  });

  it("host queryClient is used when provided", () => {
    const hostQC = new QueryClient();
    // Should not throw and client should be accessible
    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} queryClient={hostQC}>
        <Probe />
      </PensieveProvider>,
    );
    expect(screen.getByTestId("ep").textContent).toBe("https://pensieve.test");
  });
});
