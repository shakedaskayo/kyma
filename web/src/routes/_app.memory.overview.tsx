import { createFileRoute } from "@tanstack/react-router";
import { MemoryOverview } from "@/features/memory/MemoryOverview";

export const Route = createFileRoute("/_app/memory/overview")({
  component: MemoryOverview,
});
