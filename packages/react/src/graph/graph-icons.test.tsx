import { describe, it, expect } from "vitest";
import { resolveGraphIcon } from "./graph-icons";

describe("resolveGraphIcon", () => {
  it("resolves an icon for the Artifact label", () => {
    expect(resolveGraphIcon(["Artifact"], {})).not.toBeNull();
  });

  it("resolves an icon for the LogFile label", () => {
    expect(resolveGraphIcon(["LogFile"], {})).not.toBeNull();
  });
});
