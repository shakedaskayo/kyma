// Gate for control-plane-only surfaces (connectors, credentials, OAuth).
//
// Local mode (`kyma serve`, single binary over SQLite) covers memory, data,
// graph, and dashboards; connector + credential management run on the hosted
// control plane. When the connected server reports a capability as off, the
// page renders this explainer instead of firing requests that 404.

import { Cloud, ExternalLink } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { useCapabilities, type Capabilities } from "@/sdk/capabilities";

type Flag = keyof Omit<Capabilities, "mode">;

export function useCapability(feature: Flag): boolean {
  return useCapabilities()[feature];
}

export function ControlPlaneGate({
  feature,
  title,
  children,
}: {
  feature: Flag;
  /** Plural surface name for the headline, e.g. "Connectors". */
  title: string;
  children: React.ReactNode;
}) {
  const caps = useCapabilities();
  if (caps[feature]) return <>{children}</>;
  return (
    <div className="flex h-full items-center justify-center">
      <EmptyState
        icon={Cloud}
        title={`${title} run on the control plane`}
        description={
          "This kyma is the local single binary — memory, data, graph, and dashboards, zero infra. " +
          `${title} (continuous ingestion from your sources) need a hosted kyma server; ` +
          "point this CLI at one with `kyma connect <url>` or sync up with `kyma sync`."
        }
        action={
          <Button variant="outline" size="sm" asChild>
            <a href="https://shakedaskayo.github.io/kyma/" target="_blank" rel="noreferrer">
              <ExternalLink className="mr-1.5 h-3.5 w-3.5" /> Two-tier setup docs
            </a>
          </Button>
        }
      />
    </div>
  );
}
