/**
 * CapabilityGate — conditionally renders children based on server capability flags.
 *
 * ## Data shape
 * `fetchCapabilities()` (from @kyma-ai/client) returns `Capabilities`:
 *   { mode: "local"|"server", connectors: boolean, credentials: boolean,
 *     oauth: boolean, saved_views: boolean, users_admin: boolean, explore_live: boolean }
 *
 * The `capability` prop is a string key of that object (e.g. "connectors").
 *
 * ## Fail-open policy (documented here intentionally)
 * When the capabilities query is loading or has errored, CapabilityGate renders
 * children, not the unavailable fallback. Rationale: embedding must not
 * white-screen on a slow or failed /v1/capabilities call. The features the
 * gate guards will produce their own errors if genuinely absent. Only once the
 * query has succeeded AND the flag is definitively false do we show the
 * "not available on this server" card.
 */
import type { ReactNode } from "react";
import type { Capabilities } from "@kyma-ai/client";
import { useKymaCapabilities } from "../hooks/useKymaCapabilities";

export interface CapabilityGateProps {
  /** A key of the Capabilities object, e.g. "connectors", "saved_views". */
  capability: string;
  children: ReactNode;
  /** Override the default "not available on this server" card. */
  fallback?: ReactNode;
}

export function CapabilityGate({ capability, children, fallback }: CapabilityGateProps) {
  const { data, isSuccess } = useKymaCapabilities();

  // Fail-open: render children while loading or on error.
  // Only gate when we have a definitive false value.
  if (isSuccess && data != null) {
    const flag = (data as unknown as Record<string, unknown>)[capability];
    // Capability is boolean false — definitively absent
    if (flag === false) {
      if (fallback != null) return <>{fallback}</>;
      return (
        <div
          data-testid="capability-unavailable"
          className="ky-rounded-lg ky-border ky-border-border ky-bg-card ky-p-4 ky-text-sm ky-text-foreground"
        >
          <p className="ky-text-muted-foreground">
            This feature is not available on this Kyma server.
          </p>
        </div>
      );
    }
  }

  return <>{children}</>;
}

// Re-export Capabilities type for consumers that want to inspect the shape
export type { Capabilities };
