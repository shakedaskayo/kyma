import { createFileRoute } from "@tanstack/react-router";
import { RunDetail } from "@/features/dreaming/RunDetail";

export const Route = createFileRoute("/_app/memory/dreaming/$runId")({
  component: RunDetailPage,
});

function RunDetailPage() {
  const { runId } = Route.useParams();
  return <RunDetail runId={runId} />;
}
