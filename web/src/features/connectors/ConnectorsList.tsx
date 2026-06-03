import { Plug } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { useConnectors } from "./useConnectors";
import { ConnectorRow } from "./ConnectorRow";

export function ConnectorsList({ onAdd }: { onAdd: () => void }) {
  const { data: connectors, isLoading, error } = useConnectors();

  if (isLoading) {
    return <SkeletonRows rows={6} className="mx-auto max-w-3xl py-2" />;
  }

  if (error) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-destructive">
        Failed to load connectors: {(error as Error).message}
      </div>
    );
  }

  if (!connectors || connectors.length === 0) {
    return (
      <EmptyState
        icon={Plug}
        title="No connectors yet"
        description="Connect a source to continuously ingest knowledge into your context graph — code, docs, issues, and data, all in one place."
        action={<Button onClick={onAdd}>Browse connectors</Button>}
      />
    );
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-2">
      {connectors.map((c) => (
        <ConnectorRow key={c.id} connector={c} />
      ))}
    </div>
  );
}
