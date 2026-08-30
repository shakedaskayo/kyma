import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { GraphNode } from "@pensieve-ai/client";
import { PensieveProvider } from "../provider/PensieveProvider";
import { InspectorPanel } from "./InspectorPanel";
import { GraphStoreContext, createGraphStore } from "./graph-store";

// Pass-through dialog primitive (same as NodeDetailModal.test.tsx).
vi.mock("@radix-ui/react-dialog", () => {
  const Root = ({ open, children }: { open?: boolean; children: React.ReactNode }) =>
    open ? <div data-testid="dialog-root">{children}</div> : null;
  const Trigger = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Portal = ({ children }: { children: React.ReactNode }) => <>{children}</>;
  const Overlay = () => <div data-testid="dialog-overlay" />;
  const Content = ({ children }: { children: React.ReactNode }) => (
    <div data-testid="dialog-content">{children}</div>
  );
  const Header = ({ children }: { children: React.ReactNode }) => <div>{children}</div>;
  const Title = ({ children }: { children: React.ReactNode }) => <h2>{children}</h2>;
  const Close = ({ children }: { children: React.ReactNode }) => <button>{children}</button>;
  return { Root, Trigger, Portal, Overlay, Content, Header, Title, Close };
});

const NODE: GraphNode = {
  id: "memory:abc",
  labels: ["Memory"],
  properties: {
    title: "The 3-bug chain",
    content: "long body content here",
    topic_key: "claude-md:-Users-shaked-very-long-topic-key-value-that-truncates",
  },
  metadata: { created_at: "", updated_at: "", realm: "default" },
  namespace: "memory",
};

function setup() {
  const store = createGraphStore({});
  // The detail modal portals through usePortalContainer(), which requires a
  // PensieveProvider in the tree (same as NodeDetailModal.test.tsx).
  return render(
    <PensieveProvider endpoint="https://pensieve.test" auth={{ token: "tok" }}>
      <GraphStoreContext.Provider value={store}>
        <InspectorPanel node={NODE} edges={[]} nodesByCompositeId={new Map()} />
      </GraphStoreContext.Provider>
    </PensieveProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("InspectorPanel", () => {
  it("opens the detail modal when 'View details' is clicked", () => {
    setup();
    expect(screen.queryByTestId("dialog-content")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /view details/i }));
    expect(screen.getByTestId("dialog-content")).toBeTruthy();
  });

  it("expands a property row to reveal its full value", () => {
    setup();
    const expandBtn = screen.getByRole("button", { name: /expand topic_key/i });
    fireEvent.click(expandBtn);
    expect(screen.getByRole("button", { name: /collapse topic_key/i })).toBeTruthy();
  });
});
