import { afterEach, describe, expect, it, vi } from "vitest";
import { render, screen, cleanup } from "@testing-library/react";
import { PensieveProvider } from "../provider/PensieveProvider";
import { PensieveErrorBoundary } from "./PensieveErrorBoundary";

afterEach(() => cleanup());

function Bomb({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) throw new Error("test bomb");
  return <div data-testid="safe">safe</div>;
}

function Wrapper({ shouldThrow = true, onError }: { shouldThrow?: boolean; onError?: (e: unknown) => void }) {
  return (
    <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }} onError={onError}>
      <PensieveErrorBoundary>
        <Bomb shouldThrow={shouldThrow} />
      </PensieveErrorBoundary>
    </PensieveProvider>
  );
}

describe("PensieveErrorBoundary", () => {
  it("catches a rendering error and shows fallback UI with error message", () => {
    const orig = console.error;
    console.error = () => {};
    render(<Wrapper />);
    expect(screen.queryByTestId("safe")).toBeFalsy();
    // should show some error text
    expect(document.body.textContent).toContain("test bomb");
    console.error = orig;
  });

  it("shows a Retry button that resets the boundary", () => {
    const orig = console.error;
    console.error = () => {};
    render(<Wrapper />);
    const retryBtn = screen.getByRole("button", { name: /retry/i });
    expect(retryBtn).toBeTruthy();
    console.error = orig;
  });

  it("calls onError from context when an error is caught", () => {
    const orig = console.error;
    console.error = () => {};
    const onError = vi.fn();
    render(<Wrapper onError={onError} />);
    expect(onError).toHaveBeenCalledWith(expect.any(Error));
    console.error = orig;
  });

  it("renders children when fallback prop is provided and error occurs", () => {
    const orig = console.error;
    console.error = () => {};
    render(
      <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "t" }}>
        <PensieveErrorBoundary fallback={<div data-testid="custom-fb">custom fallback</div>}>
          <Bomb shouldThrow />
        </PensieveErrorBoundary>
      </PensieveProvider>,
    );
    expect(screen.getByTestId("custom-fb")).toBeTruthy();
    console.error = orig;
  });
});
