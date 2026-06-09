import { createFileRoute, Outlet } from "@tanstack/react-router";
import { MemoryHeader } from "@/features/memory/MemoryHeader";
import { MemoryStreamProvider } from "@/features/memory/stream-context";

export const Route = createFileRoute("/_app/memory")({
  component: MemoryLayout,
});

function MemoryLayout() {
  // One shared live-tail stream feeds both the sticky header's live readout and
  // the Overview's pulse rail — provided once at the layout so we never open two
  // sockets for the same firehose.
  return (
    <MemoryStreamProvider>
      <div className="flex h-full flex-col bg-surface/30">
        <MemoryHeader />
        <div className="min-h-0 flex-1">
          <Outlet />
        </div>
      </div>
    </MemoryStreamProvider>
  );
}
