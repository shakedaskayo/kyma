import { createFileRoute } from "@tanstack/react-router";
import { SyncTab } from "@/features/datasources/SyncTab";

// Memory Sync tab — Claude Code memory sync status with per-realm counters.
// Header + section tabs live in the layout route.
export const Route = createFileRoute("/_app/data-sources/sync")({
  component: () => (
    <div className="p-6">
      <SyncTab />
    </div>
  ),
});
