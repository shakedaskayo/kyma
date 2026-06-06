// ── Provider ──────────────────────────────────────────────────────────────────
export { KymaProvider } from "./provider/KymaProvider";
export type { KymaProviderProps } from "./provider/KymaProvider";
export { useKymaClient, useKymaContext } from "./provider/context";
export type { KymaContextValue } from "./provider/context";

// ── Hooks ─────────────────────────────────────────────────────────────────────
export { useKymaCapabilities } from "./hooks/useKymaCapabilities";

// ── Internal UI primitives ────────────────────────────────────────────────────
export { KymaErrorBoundary } from "./internal/KymaErrorBoundary";
export type { KymaErrorBoundaryProps } from "./internal/KymaErrorBoundary";
export { CapabilityGate } from "./internal/CapabilityGate";
export type { CapabilityGateProps } from "./internal/CapabilityGate";

// ── Themes ────────────────────────────────────────────────────────────────────
export { kymaDark, kymaLight } from "./theme/presets";
export { themeToCssVars } from "./theme/tokens";
export type { KymaTheme } from "./theme/tokens";
