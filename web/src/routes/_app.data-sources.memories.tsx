import { createFileRoute } from "@tanstack/react-router";
import { MemoriesSummaryTab } from "@/features/datasources/MemoriesSummaryTab";

// Memories tab — provenance summary (memories per source × realm). Header +
// section tabs live in the layout route.
export const Route = createFileRoute("/_app/data-sources/memories")({
  component: () => (
    <div className="p-6">
      <MemoriesSummaryTab />
    </div>
  ),
});
