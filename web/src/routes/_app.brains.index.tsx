import { createFileRoute } from "@tanstack/react-router";
import { BrainsList } from "@/features/brains/BrainsList";

export const Route = createFileRoute("/_app/brains/")({
  component: BrainsIndexPage,
});

function BrainsIndexPage() {
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto w-full max-w-6xl px-6 py-5">
        <BrainsList />
      </div>
    </div>
  );
}
