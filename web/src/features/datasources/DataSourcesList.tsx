import { Plug } from "lucide-react";
import { Button } from "@/components/ui/button";
import { SkeletonRows } from "@/components/ui/skeleton";
import type { DataSourceWatcher } from "@/sdk/datasources";
import { useDataSources } from "./useDataSources";
import { DataSourceRow } from "./DataSourceRow";

export function DataSourcesList({
  onAdd,
  watchers,
}: {
  onAdd: () => void;
  /** Node-local watchers — joined onto continuous-drive rows by id. */
  watchers?: DataSourceWatcher[];
}) {
  const { data: dataSources, isLoading, error } = useDataSources();

  if (isLoading) {
    return <SkeletonRows rows={3} className="py-2" />;
  }

  if (error) {
    return (
      <div className="flex items-center justify-center py-10 text-sm text-destructive">
        Failed to load data sources: {(error as Error).message}
      </div>
    );
  }

  if (!dataSources || dataSources.length === 0) {
    // The unified list always shows the pre-installed rows above, so an
    // empty *configured* list is an inline invite, not a full empty state.
    return (
      <div className="flex flex-col items-center gap-2 rounded-lg border border-dashed bg-background/50 px-4 py-8 text-center">
        <Plug className="h-5 w-5 text-muted-foreground" />
        <p className="text-sm text-muted-foreground">
          Connect more sources — code, docs, issues, data, or an Obsidian vault.
        </p>
        <Button size="sm" variant="outline" onClick={onAdd}>
          Browse data sources
        </Button>
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2">
      {dataSources.map((c) => (
        <DataSourceRow
          key={c.id}
          dataSource={c}
          watcher={watchers?.find((w) => w.id === `obsidian:${c.id}`)}
        />
      ))}
    </div>
  );
}
