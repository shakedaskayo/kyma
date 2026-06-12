import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DataSourcesList } from "@/features/datasources/DataSourcesList";
import { CcSyncRow, FiledropRow } from "@/features/datasources/LocalSyncRows";
import { AddDataSourceWizard } from "@/features/datasources/AddDataSourceWizard";
import { ControlPlaneGate } from "@/features/capabilities/ControlPlaneGate";
import { useDataSourceWatchers } from "@/features/datasources/useDataSources";

// The unified Data Sources page: one list with the pre-installed Claude Code
// memory sync on top, then every configured source (continuous vault watchers
// and scheduled connectors alike) and any filedrop watchers. The page title
// lives in the `_app.data-sources.tsx` layout; the detail in `$id`.
//
// NOTE: the engine's OAuth callback fallback redirect lands on
// /data-sources/new (see crates/kyma-datasources/src/oauth/handler.rs), which
// matches the sibling $id route with id="new" — the actual OAuth result is
// delivered via postMessage/polling in useOAuthConnect, not via that URL.
export const Route = createFileRoute("/_app/data-sources/")({
  component: DataSourcesPage,
});

function DataSourcesPage() {
  const navigate = useNavigate();
  const [addOpen, setAddOpen] = useState(false);
  const { data: watchers } = useDataSourceWatchers();
  const filedrops = (watchers ?? []).filter((w) => w.kind === "filedrop");

  return (
    <div className="p-6">
      <ControlPlaneGate feature="data_sources" title="Data Sources">
        <div className="mx-auto flex max-w-3xl flex-col gap-2">
          <div className="mb-1 flex items-center justify-end">
            <Button onClick={() => setAddOpen(true)}>
              <Plus className="mr-1.5 h-4 w-4" /> Add data source
            </Button>
          </div>
          <CcSyncRow />
          {filedrops.map((w) => (
            <FiledropRow key={w.id} watcher={w} />
          ))}
          <DataSourcesList onAdd={() => setAddOpen(true)} watchers={watchers} />
        </div>
      </ControlPlaneGate>

      <AddDataSourceWizard
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onCreated={(id) => navigate({ to: "/data-sources/$id", params: { id } })}
      />
    </div>
  );
}
