import { createFileRoute, redirect } from "@tanstack/react-router";

// The Memory Sync tab merged into the unified /data-sources page (the
// Claude Code row expands to the per-realm sync table).
export const Route = createFileRoute("/_app/data-sources/sync")({
  beforeLoad: () => {
    throw redirect({ to: "/data-sources" });
  },
});
