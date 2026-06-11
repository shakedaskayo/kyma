import { createFileRoute } from "@tanstack/react-router";

// Stub — Task 16 replaces this with the real Memories surface.
export const Route = createFileRoute("/_app/data-sources/memories")({
  component: () => (
    <p className="p-6 text-sm text-muted-foreground">Memories — coming in the next commit.</p>
  ),
});
