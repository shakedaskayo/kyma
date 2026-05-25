import { createFileRoute } from "@tanstack/react-router";
import { GraphView } from "@/features/graph/GraphView";

export const Route = createFileRoute("/_app/graph")({
  component: GraphView,
});
