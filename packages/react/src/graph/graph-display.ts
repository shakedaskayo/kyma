/**
 * Pure display logic for the Sigma renderer: zoom LOD tiers and the
 * quiet-arrows/loud-focus styling contract. Sigma's node/edge reducers call
 * these every frame, so everything here must be allocation-light.
 *
 * Tiers (sigma camera ratio grows when zooming OUT; ratio 1 ≈ fitted view):
 *   far  — whole-galaxy view: dots + hulls, landmark labels only
 *   mid  — arrows + icons visible, focused labels
 *   near — all labels, edge labels on focus
 *
 * Amendment (2026-06-11): at-rest edges are NEUTRAL hairlines (not
 * family-colored); family color only in the loud state. The `neutral` field
 * signals the renderer to pick rgba(148,163,184,alpha) instead of the family
 * color. Loud/dim branches all set neutral=false.
 */

export type LodTier = "far" | "mid" | "near";

export const FAR_RATIO = 0.8;
export const NEAR_RATIO = 0.25;

export function lodTier(cameraRatio: number): LodTier {
  if (cameraRatio >= FAR_RATIO) return "far";
  if (cameraRatio >= NEAR_RATIO) return "mid";
  return "near";
}

export interface DisplayCtx {
  tier: LodTier;
  /** Hovered or selected composite id (hover wins) — the "loud" center. */
  focusId: string | null;
  /** focusId + its 1-hop neighbors. Null when no focus. */
  neighborhood: Set<string> | null;
  /** Composite ids matching the active search. Null when no search. */
  searchMatches: Set<string> | null;
  relTypeFilter: string | null;
  showEdgeLabels: boolean;
  isDark: boolean;
  /** Focus-neighborhood isolation (double-click): only these ids render. */
  focusModeIds: Set<string> | null;
}

export interface NodeDisplay {
  hidden: boolean;
  dimmed: boolean;
  highlighted: boolean;
  label: string | null;
}

export function nodeDisplay(
  id: string,
  attrs: { label: string; size: number },
  ctx: DisplayCtx,
): NodeDisplay {
  if (ctx.focusModeIds && !ctx.focusModeIds.has(id)) {
    return { hidden: true, dimmed: false, highlighted: false, label: null };
  }
  const isMatch = ctx.searchMatches?.has(id) ?? false;
  const inHood = ctx.neighborhood?.has(id) ?? false;
  const hasFocus = ctx.focusId !== null;
  const hasSearch = ctx.searchMatches !== null;
  const dimmed = (hasFocus && !inHood) || (hasSearch && !isMatch);
  const highlighted = id === ctx.focusId || isMatch;
  // Labels: always for highlighted/neighborhood; by tier otherwise.
  const label =
    highlighted || (inHood && ctx.tier !== "far")
      ? attrs.label
      : dimmed
        ? null
        : ctx.tier === "near"
          ? attrs.label
          : ctx.tier === "mid" && attrs.size >= 8 // landmark-ish nodes
            ? attrs.label
            : null;
  return { hidden: false, dimmed, highlighted, label };
}

export interface EdgeDisplay {
  hidden: boolean;
  alpha: number;
  size: number;
  label: string | null;
  /** True → bold arrowhead (loud); false → small quiet arrowhead. */
  loud: boolean;
  /**
   * True → renderer uses neutral hairline color rgba(148,163,184,alpha)
   * instead of the relationship family color. Only true in the at-rest state.
   */
  neutral: boolean;
}

export function edgeDisplay(
  _id: string,
  attrs: { size: number; relType: string },
  source: string,
  target: string,
  ctx: DisplayCtx,
): EdgeDisplay {
  if (ctx.focusModeIds && !(ctx.focusModeIds.has(source) && ctx.focusModeIds.has(target))) {
    return { hidden: true, alpha: 0, size: 0, label: null, loud: false, neutral: false };
  }
  // Relationship-type isolation deliberately takes priority over hover/selection:
  // when a type is isolated, the focus-incident "loud" branch below never runs.
  if (ctx.relTypeFilter) {
    if (attrs.relType !== ctx.relTypeFilter) {
      return {
        hidden: false,
        alpha: 0.04,
        size: attrs.size * 0.8,
        label: null,
        loud: false,
        neutral: false,
      };
    }
    return {
      hidden: false,
      alpha: 0.95,
      size: attrs.size * 1.6,
      label: ctx.tier === "near" ? attrs.relType : null,
      loud: true,
      neutral: false,
    };
  }
  const hasFocus = ctx.focusId !== null;
  const incident = hasFocus && (source === ctx.focusId || target === ctx.focusId);
  if (incident) {
    return {
      hidden: false,
      alpha: 0.85,
      size: Math.max(attrs.size * 1.8, 2),
      label: ctx.tier === "far" ? null : attrs.relType,
      loud: true,
      neutral: false,
    };
  }
  if (hasFocus) {
    return {
      hidden: false,
      alpha: 0.05,
      size: attrs.size * 0.8,
      label: null,
      loud: false,
      neutral: false,
    };
  }
  // At rest: neutral hairlines — amendment override.
  // alpha: 0.1 far; 0.15 mid/near dark; 0.22 mid/near light. (Cooled after
  // live review: converging fans run hot, especially on the dark theme.)
  const alpha =
    ctx.tier === "far" ? 0.1 : ctx.isDark ? 0.15 : 0.22;
  return {
    hidden: false,
    alpha,
    size: attrs.size,
    label: ctx.showEdgeLabels && ctx.tier === "near" ? attrs.relType : null,
    loud: false,
    neutral: true,
  };
}
