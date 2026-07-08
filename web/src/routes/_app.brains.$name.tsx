import { createFileRoute } from "@tanstack/react-router";
import { BrainDetail } from "@/features/brains/BrainDetail";

export const Route = createFileRoute("/_app/brains/$name")({
  component: BrainDetailPage,
});

function BrainDetailPage() {
  const { name } = Route.useParams();
  return (
    <div className="mx-auto h-full w-full max-w-6xl">
      <BrainDetail name={name} />
    </div>
  );
}
