import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ConnectorsList } from "@/features/connectors/ConnectorsList";
import { AddConnectorWizard } from "@/features/connectors/AddConnectorWizard";

export const Route = createFileRoute("/_app/connectors")({
  component: ConnectorsListPage,
});

function ConnectorsListPage() {
  const navigate = useNavigate();
  const [addOpen, setAddOpen] = useState(false);

  return (
    <div className="flex h-full flex-col bg-muted/20">
      {/* Header */}
      <div className="flex items-center justify-between border-b bg-background px-6 py-4">
        <div>
          <h1 className="text-lg font-semibold">Connectors</h1>
          <p className="text-sm text-muted-foreground">
            Continuously ingest knowledge from your sources into the context graph.
          </p>
        </div>
        <Button onClick={() => setAddOpen(true)}>
          <Plus className="mr-1.5 h-4 w-4" /> Add connector
        </Button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-6">
        <ConnectorsList onAdd={() => setAddOpen(true)} />
      </div>

      <AddConnectorWizard
        open={addOpen}
        onClose={() => setAddOpen(false)}
        onCreated={(id) => navigate({ to: "/connectors/$id", params: { id } })}
      />
    </div>
  );
}
