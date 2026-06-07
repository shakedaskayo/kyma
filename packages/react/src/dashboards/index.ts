// ── Primary public component ──────────────────────────────────────────────────
export { KymaDashboard } from "./KymaDashboard";
export type { KymaDashboardProps } from "./KymaDashboard";

// ── Types re-exports ──────────────────────────────────────────────────────────
export type { TimeRange } from "../query/time-range/time-range-types";
export type { Dashboard, DashboardWithPanels, DashboardPanel } from "@kyma-ai/client";

// ── Sub-components (for advanced composition) ─────────────────────────────────
export { PanelCard } from "./PanelCard";
export { AddPanelModal } from "./AddPanelModal";
export type { PanelDraft } from "./AddPanelModal";
