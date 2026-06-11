import { createFileRoute } from "@tanstack/react-router";
import { WatchersTab } from "@/features/datasources/WatchersTab";

// File Watchers tab — node-local filedrop + cc-sync watchers and their
// heartbeat/scan health. Header + section tabs live in the layout route.
export const Route = createFileRoute("/_app/data-sources/watchers")({
  component: () => (
    <div className="p-6">
      <WatchersTab />
    </div>
  ),
});
