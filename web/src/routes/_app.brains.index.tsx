import { createFileRoute } from "@tanstack/react-router";
import { BrainsList } from "@/features/brains/BrainsList";

export const Route = createFileRoute("/_app/brains/")({
  component: BrainsList,
});
