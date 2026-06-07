// ── Provider ──────────────────────────────────────────────────────────────────
export { KymaProvider } from "./provider/KymaProvider";
export type { KymaProviderProps } from "./provider/KymaProvider";
export { useKymaClient, useKymaContext } from "./provider/context";
export type { KymaContextValue } from "./provider/context";

// ── Hooks ─────────────────────────────────────────────────────────────────────
export { useKymaCapabilities } from "./hooks/useKymaCapabilities";
export { useKymaGraph } from "./hooks/useKymaGraph";
export type { UseKymaGraphArgs, UseKymaGraphResult } from "./hooks/useKymaGraph";
export { useKymaQuery } from "./hooks/useKymaQuery";
export type { KymaQueryArgs, UseKymaQueryResult } from "./hooks/useKymaQuery";
export { useKymaDiscover } from "./hooks/useKymaDiscover";
export type { UseKymaDiscoverResult } from "./hooks/useKymaDiscover";
export { useKymaDashboards } from "./hooks/useKymaDashboards";
export type { UseKymaDashboardsResult } from "./hooks/useKymaDashboards";
export { useKymaAgent } from "./hooks/useKymaAgent";
export type { AgentMessage, AgentStatus, UseKymaAgentArgs, UseKymaAgentResult } from "./hooks/useKymaAgent";

// ── Re-exported types from @kyma-ai/client ────────────────────────────────────
// Commonly-needed types so consumers don't need to also import @kyma-ai/client.
export type {
  GraphNode,
  GraphRelationship,
  GraphStats,
  GraphPayload,
  SearchHits,
  Capabilities,
  Dashboard,
  DashboardWithPanels,
  DashboardPanel,
  Frame,
  Column,
  ColKind,
} from "@kyma-ai/client";

// ── Internal UI primitives ────────────────────────────────────────────────────
export { KymaErrorBoundary } from "./internal/KymaErrorBoundary";
export type { KymaErrorBoundaryProps } from "./internal/KymaErrorBoundary";
export { CapabilityGate } from "./internal/CapabilityGate";
export type { CapabilityGateProps } from "./internal/CapabilityGate";

// ── Dashboards ────────────────────────────────────────────────────────────────
export { KymaDashboard } from "./dashboards/KymaDashboard";
export type { KymaDashboardProps } from "./dashboards/KymaDashboard";

// ── Themes ────────────────────────────────────────────────────────────────────
export { kymaDark, kymaLight } from "./theme/presets";
export { themeToCssVars } from "./theme/tokens";
export type { KymaTheme } from "./theme/tokens";
