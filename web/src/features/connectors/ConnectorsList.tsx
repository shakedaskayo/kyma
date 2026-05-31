import { Plug } from "lucide-react";
import { useConnectors } from "./useConnectors";
import { ConnectorRow } from "./ConnectorRow";

export function ConnectorsList({ onAdd }: { onAdd: () => void }) {
  const { data: connectors, isLoading, error } = useConnectors();

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-muted-foreground animate-pulse">
        Loading connectors…
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-destructive">
        Failed to load connectors: {(error as Error).message}
      </div>
    );
  }

  if (!connectors || connectors.length === 0) {
    return <EmptyState onAdd={onAdd} />;
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-2">
      {connectors.map((c) => (
        <ConnectorRow key={c.id} connector={c} />
      ))}
    </div>
  );
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center py-20 text-center">
      <div className="mb-4 rounded-full bg-primary/10 p-5">
        <Plug className="h-10 w-10 text-primary/70" />
      </div>
      <h2 className="text-base font-semibold">No connectors yet</h2>
      <p className="mt-1 max-w-sm text-sm text-muted-foreground">
        Connect a source to continuously ingest knowledge into your context graph.
        Start with a GitHub repository.
      </p>
      <button
        onClick={onAdd}
        className="mt-4 inline-flex items-center gap-1.5 rounded-md bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition hover:bg-primary/90"
      >
        Add connector
      </button>
    </div>
  );
}
