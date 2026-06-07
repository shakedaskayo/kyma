import { createFileRoute } from "@tanstack/react-router";
import { DreamingPanel } from "@/features/dreaming/DreamingPanel";

export const Route = createFileRoute("/_app/memory/dreaming/")({
  component: DreamingPanel,
});
