import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/_app/explore")({
  component: () => <div className="p-6 text-sm text-muted-foreground">Explore route — workspace lands here in Phase 8.</div>,
});
