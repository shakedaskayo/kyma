import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { GraphNode } from "@kyma-ai/client";
import { KymaProvider } from "../provider/KymaProvider";
import { NodeDetailModal } from "./NodeDetailModal";

// jsdom can't run Radix portals/animations — replace the dialog primitive with
// simple pass-through elements (same approach as KymaDashboard.test.tsx).
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

function makeNode(properties: Record<string, unknown>): GraphNode {
  return {
    id: "memory:abc",
    labels: ["Memory"],
    properties,
    metadata: { created_at: "", updated_at: "", realm: "default" },
    namespace: "memory",
  };
}

function renderModal(node: GraphNode) {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          object_path: "artifacts/t/logs/build.log",
          size_bytes: 5,
          offset: 0,
          returned_bytes: 5,
          eof: true,
          content: "hello",
        }),
        { status: 200, headers: new Headers({ "content-type": "application/json" }) },
      ),
    ),
  );
  return render(
    <KymaProvider endpoint="https://kyma.test" auth={{ token: "tok" }}>
      <NodeDetailModal node={node} open onClose={() => {}} />
    </KymaProvider>,
  );
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("NodeDetailModal", () => {
  it("renders content as markdown and lists all properties untruncated", () => {
    renderModal(makeNode({ content: "# Heading\n\nbody text", importance: 0.7 }));
    expect(screen.getByRole("heading", { level: 1, name: "Heading" })).toBeTruthy();
    expect(screen.getByText("importance")).toBeTruthy();
    expect(screen.getByText("0.7")).toBeTruthy();
  });

  it("omits the source section when there is no object_path", () => {
    renderModal(makeNode({ content: "x" }));
    expect(screen.queryByText(/source file/i)).toBeNull();
  });

  it("shows the source section when the node has an object_path", () => {
    renderModal(makeNode({ content: "x", object_path: "artifacts/t/logs/build.log" }));
    expect(screen.getByText(/source file/i)).toBeTruthy();
  });
});
