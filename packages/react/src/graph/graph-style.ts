/**
 * Visual language for the graph explorer — relationship families, color/size
 * helpers, and level-of-detail thresholds.
 * Adapted from web/src/features/graph/graph-style.ts.
 * Uses internal data-palette (no web imports).
 */
import { PALETTE } from "../internal/data-palette";
import { getLabelColor, getRelationshipColor } from "@kyma-ai/client";

export type RelFamily =
  | "structural"
  | "dependency"
  | "reference"
  | "membership"
  | "access"
  | "authorship"
  | "other";

/** Coarse family for a raw relationship type — collapses 50+ types into ~6. */
export function relationshipFamily(type: string): RelFamily {
  const t = (type ?? "").toUpperCase();
  // CI / GitHub Actions spine: repo→run→job→log is structural; the run→branch
  // link is membership.
  if (/^(HAS_RUN|RUN_CONTAINS_JOB|JOB_HAS_LOG)$/.test(t)) return "structural";
  if (t === "RUN_ON_BRANCH") return "membership";
  if (/CONTAIN|^HAS_|OWNS|EXPOSE/.test(t)) return "structural";
  if (/DEPEND|USE|RUN[S_]|CONNECT|INDEX|CONSTRAIN|PROVIDE/.test(t)) return "dependency";
  if (/REFERENCE|RELATED|LINK|POINTS?_TO/.test(t)) return "reference";
  if (/MEMBER|BELONG/.test(t)) return "membership";
  if (/ACCESS|MANAGE|MONITOR|PERMISSION/.test(t)) return "access";
  if (/CREATED|AUTHOR|MODIFIED/.test(t)) return "authorship";
  return "other";
}

export const FAMILY_COLORS: Record<RelFamily, string> = {
  structural: PALETTE.cyan,
  dependency: PALETTE.amber,
  reference: PALETTE.slate,
  membership: PALETTE.teal,
  access: PALETTE.rose,
  authorship: PALETTE.emerald,
  other: PALETTE.indigo,
};

export const FAMILY_LABEL: Record<RelFamily, string> = {
  structural: "Structural",
  dependency: "Dependency",
  reference: "Reference",
  membership: "Membership",
  access: "Access",
  authorship: "Authorship",
  other: "Other",
};

/** Family color for a relationship type (the cohesive edge color by default). */
export function getRelationshipFamilyColor(type: string): string {
  return FAMILY_COLORS[relationshipFamily(type)];
}

/** Re-export the per-label node color so consumers import one module. */
export { getLabelColor, getRelationshipColor };

/** Lighten a #RRGGBB hex toward white by amount (0..1). Used for node-core highlights. */
export function lighten(hex: string, amt: number): string {
  let h = hex.replace("#", "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  const mix = (c: number) => Math.round(c + (255 - c) * amt);
  return `rgb(${mix(r)}, ${mix(g)}, ${mix(b)})`;
}

/** Darken a #RRGGBB hex toward black by amount (0..1). Used for node rims/depth. */
export function darken(hex: string, amt: number): string {
  let h = hex.replace("#", "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h, 16);
  const r = (n >> 16) & 255;
  const g = (n >> 8) & 255;
  const b = n & 255;
  const mix = (c: number) => Math.round(c * (1 - amt));
  return `rgb(${mix(r)}, ${mix(g)}, ${mix(b)})`;
}

/** Add alpha to a #RRGGBB hex → rgba() (local copy to avoid a cross-module dep cycle). */
export function alpha(hex: string, a: number): string {
  let h = hex.replace("#", "");
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  const n = parseInt(h, 16);
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${a})`;
}

/**
 * Node radius (graph units) from degree centrality. sqrt curve so area scales
 * with degree; clamped so a single mega-hub doesn't flatten everyone. capDeg is
 * typically a high percentile of the degree distribution.
 */
export function radiusForDegree(degree: number, capDeg: number, sizeByDegree: boolean): number {
  const base = 5.5;
  if (!sizeByDegree) return base + 1;
  const cap = Math.max(capDeg, 1);
  const t = Math.sqrt(Math.min(degree, cap) / cap);
  return base + 8.5 * t; // 5.5 → 14 graph units (bigger, so icons read sooner)
}

/** Level-of-detail zoom thresholds (react-force-graph globalScale units). */
export const LOD = {
  /** Below this, only landmark / selected / hovered / search labels show. */
  labelAll: 1.3,
  /** Below this, hide even landmark labels (pure dots). */
  labelLandmark: 0.45,
  /** Show type icons inside large nodes above this zoom. */
  icon: 2.2,
  /** Show edge labels above this zoom (when the toggle is on). */
  edgeLabel: 1.1,
};
