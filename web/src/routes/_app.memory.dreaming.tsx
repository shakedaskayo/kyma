import { createFileRoute, Outlet, useMatch } from "@tanstack/react-router";
import { DreamingToolbar } from "@/features/dreaming/DreamingToolbar";
import { NodesStrip } from "@/features/dreaming/NodesStrip";
import { RunsList } from "@/features/dreaming/RunsList";

// Layout route for /memory/dreaming — hosts the persistent two-pane split:
// a runs list on the left (always visible) and the selected run's detail (or
// an empty placeholder) on the right via the child route's Outlet.
export const Route = createFileRoute("/_app/memory/dreaming")({
  component: DreamingLayout,
});

function DreamingLayout() {
  const runIdMatch = useMatch({ from: "/_app/memory/dreaming/$runId", shouldThrow: false });
  const selectedId = runIdMatch?.params.runId ?? null;

  return (
    <div className="flex h-full flex-col">
      <DreamingToolbar />
      <NodesStrip />
      <div className="flex min-h-0 flex-1">
        <div className="w-96 shrink-0 border-r border-border/60">
          <RunsList selectedId={selectedId} />
        </div>
        <div className="min-w-0 flex-1">
          <Outlet />
        </div>
      </div>
    </div>
  );
}
