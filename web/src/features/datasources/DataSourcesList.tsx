import { Plug } from "lucide-react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { SkeletonRows } from "@/components/ui/skeleton";
import { useDataSources } from "./useDataSources";
import { DataSourceRow } from "./DataSourceRow";

export function DataSourcesList({ onAdd }: { onAdd: () => void }) {
  const { data: dataSources, isLoading, error } = useDataSources();

  if (isLoading) {
    return <SkeletonRows rows={6} className="mx-auto max-w-3xl py-2" />;
  }

  if (error) {
    return (
      <div className="flex items-center justify-center py-20 text-sm text-destructive">
        Failed to load data sources: {(error as Error).message}
      </div>
    );
  }

  if (!dataSources || dataSources.length === 0) {
    return (
      <EmptyState
        icon={Plug}
        title="No data sources yet"
        description="Connect a source to continuously ingest knowledge into your context graph — code, docs, issues, and data, all in one place."
        action={<Button onClick={onAdd}>Browse data sources</Button>}
      />
    );
  }

  return (
    <div className="mx-auto flex max-w-3xl flex-col gap-2">
      {dataSources.map((c) => (
        <DataSourceRow key={c.id} dataSource={c} />
      ))}
    </div>
  );
}
