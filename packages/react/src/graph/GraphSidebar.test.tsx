import { describe, it, expect } from "vitest";
import { artifactViewerPath } from "./GraphSidebar";

const node = (labels: string[], properties: Record<string, unknown>) =>
  ({ id: "n", labels, properties }) as Parameters<typeof artifactViewerPath>[0];

describe("artifactViewerPath", () => {
  it("returns the object_path for an Artifact-labeled node", () => {
    expect(
      artifactViewerPath(node(["Artifact"], { object_path: "a/b.log" })),
    ).toBe("a/b.log");
  });

  it("still fires for legacy LogFile-labeled nodes", () => {
    expect(
      artifactViewerPath(node(["LogFile"], { object_path: "a/b.log" })),
    ).toBe("a/b.log");
  });

  it("reads object_path nested in a JSON props blob", () => {
    expect(
      artifactViewerPath(node(["Artifact"], { props: '{"object_path":"x/y.log"}' })),
    ).toBe("x/y.log");
  });

  it("returns null for non-artifact nodes", () => {
    expect(artifactViewerPath(node(["Job"], { object_path: "a/b.log" }))).toBeNull();
  });

  it("returns null when there is no object_path", () => {
    expect(artifactViewerPath(node(["Artifact"], {}))).toBeNull();
  });
});
