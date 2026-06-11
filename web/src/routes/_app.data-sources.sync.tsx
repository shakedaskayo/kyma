import { createFileRoute } from "@tanstack/react-router";

// Stub — Task 16 replaces this with the real Memory Sync surface.
export const Route = createFileRoute("/_app/data-sources/sync")({
  component: () => (
    <p className="p-6 text-sm text-muted-foreground">Memory sync — coming in the next commit.</p>
  ),
});
