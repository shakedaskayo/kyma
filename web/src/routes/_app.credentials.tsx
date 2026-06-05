import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CredentialsList } from "@/features/credentials/CredentialsList";
import { AddCredentialDialog } from "@/features/credentials/AddCredentialDialog";
import { ControlPlaneGate, useCapability } from "@/features/capabilities/ControlPlaneGate";

export const Route = createFileRoute("/_app/credentials")({
  component: CredentialsPage,
});

function CredentialsPage() {
  const [addOpen, setAddOpen] = useState(false);
  const supported = useCapability("credentials");
  return (
    <div className="flex h-full flex-col bg-muted/20">
      <div className="flex items-center justify-between border-b bg-background px-6 py-4">
        <div>
          <h1 className="text-lg font-semibold">Credentials</h1>
          <p className="text-sm text-muted-foreground">
            Typed, encrypted secrets — reused by connectors, MCP servers, and agents.
          </p>
        </div>
        {supported && (
          <Button onClick={() => setAddOpen(true)}>
            <Plus className="mr-1.5 h-4 w-4" /> Add credential
          </Button>
        )}
      </div>
      <div className="flex-1 overflow-y-auto p-6">
        <ControlPlaneGate feature="credentials" title="Credentials">
          <CredentialsList onAdd={() => setAddOpen(true)} />
        </ControlPlaneGate>
      </div>
      <AddCredentialDialog open={addOpen} onClose={() => setAddOpen(false)} />
    </div>
  );
}
