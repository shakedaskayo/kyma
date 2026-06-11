import { describe, expect, it } from "vitest";
import {
  FAR_RATIO,
  NEAR_RATIO,
  edgeDisplay,
  lodTier,
  nodeDisplay,
  type DisplayCtx,
} from "./graph-display";

const baseCtx = (over: Partial<DisplayCtx> = {}): DisplayCtx => ({
  tier: "mid",
  focusId: null,
  neighborhood: null,
  searchMatches: null,
  relTypeFilter: null,
  showEdgeLabels: false,
  isDark: true,
  focusModeIds: null,
  ...over,
});

describe("lodTier", () => {
  it("maps camera ratio to tiers", () => {
    expect(lodTier(FAR_RATIO + 1)).toBe("far");
    expect(lodTier((FAR_RATIO + NEAR_RATIO) / 2)).toBe("mid");
    expect(lodTier(NEAR_RATIO / 2)).toBe("near");
  });
});

describe("nodeDisplay (quiet/loud)", () => {
  const attrs = { label: "svc", color: "#22d3ee", size: 6, image: undefined };

  it("at rest: keeps color, hides label when far", () => {
    const d = nodeDisplay("a", attrs, baseCtx({ tier: "far" }));
    expect(d.label).toBeNull();
    expect(d.dimmed).toBe(false);
  });

  it("dims nodes outside the focus neighborhood", () => {
    const ctx = baseCtx({ focusId: "x", neighborhood: new Set(["x", "y"]) });
    expect(nodeDisplay("a", attrs, ctx).dimmed).toBe(true);
    expect(nodeDisplay("y", attrs, ctx).dimmed).toBe(false);
    expect(nodeDisplay("x", attrs, ctx).highlighted).toBe(true);
  });

  it("search matches stay lit and labeled even when far", () => {
    const ctx = baseCtx({ tier: "far", searchMatches: new Set(["a"]) });
    const d = nodeDisplay("a", attrs, ctx);
    expect(d.dimmed).toBe(false);
    expect(d.label).toBe("svc");
  });

  it("focus mode hides everything outside the focus set", () => {
    const ctx = baseCtx({ focusModeIds: new Set(["a"]) });
    expect(nodeDisplay("b", attrs, ctx).hidden).toBe(true);
    expect(nodeDisplay("a", attrs, ctx).hidden).toBe(false);
  });
});

describe("edgeDisplay (quiet/loud)", () => {
  const attrs = { color: "#f59e0b", size: 1, relType: "DEPENDS_ON" };

  it("at rest: low alpha, no label, thin, neutral=true", () => {
    const d = edgeDisplay("e1", attrs, "a", "b", baseCtx());
    expect(d.alpha).toBeLessThan(0.5);
    expect(d.label).toBeNull();
    expect(d.neutral).toBe(true);
  });

  it("at rest far tier: alpha=0.1, neutral=true", () => {
    const d = edgeDisplay("e1", attrs, "a", "b", baseCtx({ tier: "far" }));
    expect(d.alpha).toBe(0.1);
    expect(d.neutral).toBe(true);
  });

  it("at rest mid/near dark mode: alpha=0.15, neutral=true", () => {
    const dMid = edgeDisplay("e1", attrs, "a", "b", baseCtx({ tier: "mid", isDark: true }));
    expect(dMid.alpha).toBe(0.15);
    expect(dMid.neutral).toBe(true);
    const dNear = edgeDisplay("e1", attrs, "a", "b", baseCtx({ tier: "near", isDark: true }));
    expect(dNear.alpha).toBe(0.15);
    expect(dNear.neutral).toBe(true);
  });

  it("at rest mid/near light mode: alpha=0.22, neutral=true", () => {
    const d = edgeDisplay("e1", attrs, "a", "b", baseCtx({ tier: "mid", isDark: false }));
    expect(d.alpha).toBe(0.22);
    expect(d.neutral).toBe(true);
  });

  it("loud when incident to focus: bold + labeled, neutral=false", () => {
    const ctx = baseCtx({ focusId: "a", neighborhood: new Set(["a", "b"]) });
    const d = edgeDisplay("e1", attrs, "a", "b", ctx);
    expect(d.alpha).toBeGreaterThanOrEqual(0.85);
    expect(d.size).toBeGreaterThan(attrs.size);
    expect(d.label).toBe("DEPENDS_ON");
    expect(d.loud).toBe(true);
    expect(d.neutral).toBe(false);
  });

  it("near-mute when focus exists elsewhere", () => {
    const ctx = baseCtx({ focusId: "x", neighborhood: new Set(["x"]) });
    expect(edgeDisplay("e1", attrs, "a", "b", ctx).alpha).toBeLessThanOrEqual(0.08);
  });

  it("relationship-type isolation filter", () => {
    const ctx = baseCtx({ relTypeFilter: "CONTAINS" });
    expect(edgeDisplay("e1", attrs, "a", "b", ctx).alpha).toBeLessThanOrEqual(0.06);
    // muted edge: neutral=false (it's explicitly filtered, not at rest)
    expect(edgeDisplay("e1", attrs, "a", "b", ctx).neutral).toBe(false);
    const d = edgeDisplay("e1", { ...attrs, relType: "CONTAINS" }, "a", "b", ctx);
    expect(d.alpha).toBeGreaterThanOrEqual(0.85);
    // loud edge: neutral=false
    expect(d.neutral).toBe(false);
  });
});
