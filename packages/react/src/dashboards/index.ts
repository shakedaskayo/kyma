// ── Primary public component ──────────────────────────────────────────────────
export { PensieveDashboard } from "./PensieveDashboard";
export type { PensieveDashboardProps } from "./PensieveDashboard";

// ── Types re-exports ──────────────────────────────────────────────────────────
export type { TimeRange } from "../query/time-range/time-range-types";
export type { Dashboard, DashboardWithPanels, DashboardPanel } from "@pensieve-ai/client";

// ── Sub-components (for advanced composition) ─────────────────────────────────
export { PanelCard } from "./PanelCard";
export { AddPanelModal } from "./AddPanelModal";
export type { PanelDraft } from "./AddPanelModal";
