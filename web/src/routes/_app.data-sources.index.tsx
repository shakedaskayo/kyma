import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { DataSourcesList } from "@/features/datasources/DataSourcesList";
import { AddDataSourceWizard } from "@/features/datasources/AddDataSourceWizard";
import { ControlPlaneGate } from "@/features/capabilities/ControlPlaneGate";

// The Sources tab: the list of configured data sources. The page title and
// section tabs live in the `_app.data-sources.tsx` layout; this route only
// owns the list content + the Add wizard.
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

  return (
    <div className="p-6">
      <ControlPlaneGate feature="data_sources" title="Data Sources">
        <div className="mb-4 flex justify-end">
          <Button onClick={() => setAddOpen(true)}>
            <Plus className="mr-1.5 h-4 w-4" /> Add data source
          </Button>
        </div>
        <DataSourcesList onAdd={() => setAddOpen(true)} />
      </ControlPlaneGate>

      <AddDataSourceWizard
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onCreated={(id) => navigate({ to: "/data-sources/$id", params: { id } })}
      />
    </div>
  );
}
