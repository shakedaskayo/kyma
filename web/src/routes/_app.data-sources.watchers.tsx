import { createFileRoute } from "@tanstack/react-router";

// Stub — Task 16 replaces this with the real File Watchers surface.
export const Route = createFileRoute("/_app/data-sources/watchers")({
  component: () => (
    <p className="p-6 text-sm text-muted-foreground">File watchers — coming in the next commit.</p>
  ),
});
