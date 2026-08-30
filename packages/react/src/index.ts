// ── Provider ──────────────────────────────────────────────────────────────────
export { PensieveProvider } from "./provider/PensieveProvider";
export type { PensieveProviderProps } from "./provider/PensieveProvider";
export { usePensieveClient, usePensieveContext } from "./provider/context";
export type { PensieveContextValue } from "./provider/context";

// ── Hooks ─────────────────────────────────────────────────────────────────────
export { usePensieveCapabilities } from "./hooks/usePensieveCapabilities";
export { usePensieveGraph } from "./hooks/usePensieveGraph";
export type { UsePensieveGraphArgs, UsePensieveGraphResult } from "./hooks/usePensieveGraph";
export { usePensieveQuery } from "./hooks/usePensieveQuery";
export { usePensieveSearch } from "./hooks/usePensieveSearch";
export type { UsePensieveSearchResult } from "./hooks/usePensieveSearch";
export type { PensieveQueryArgs, UsePensieveQueryResult } from "./hooks/usePensieveQuery";
export { usePensieveDiscover } from "./hooks/usePensieveDiscover";
export type { UsePensieveDiscoverResult } from "./hooks/usePensieveDiscover";
export { usePensieveDashboards } from "./hooks/usePensieveDashboards";
export type { UsePensieveDashboardsResult } from "./hooks/usePensieveDashboards";
export { usePensieveAgent } from "./hooks/usePensieveAgent";
export type { AgentMessage, AgentStatus, UsePensieveAgentArgs, UsePensieveAgentResult } from "./hooks/usePensieveAgent";
export { usePensieveHealth } from "./hooks/usePensieveHealth";

// ── Re-exported types from @pensieve-ai/client ────────────────────────────────────
// Commonly-needed types so consumers don't need to also import @pensieve-ai/client.
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
} from "@pensieve-ai/client";

// ── Internal UI primitives ────────────────────────────────────────────────────
export { PensieveErrorBoundary } from "./internal/PensieveErrorBoundary";
export type { PensieveErrorBoundaryProps } from "./internal/PensieveErrorBoundary";
export { CapabilityGate } from "./internal/CapabilityGate";
export type { CapabilityGateProps } from "./internal/CapabilityGate";

// View components are NOT re-exported from the root on purpose: each lives on
// its own subpath (`@pensieve-ai/react/graph`, `/query`, `/discover`, `/dashboards`,
// `/agent`) so a consumer importing only the provider/hooks never pulls a
// view's heavy dependency (echarts, monaco, force-graph, ai-sdk) into their
// static graph. Dashboard types that hooks return are re-exported above.

// ── Themes ────────────────────────────────────────────────────────────────────
export { pensieveDark, pensieveLight } from "./theme/presets";
export { themeToCssVars } from "./theme/tokens";
export type { PensieveTheme } from "./theme/tokens";
