import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/settings")({
  component: () => <div className="p-6 text-sm text-muted-foreground">Settings route — placeholder for Phase 8.</div>,
});
