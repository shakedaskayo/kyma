import { createFileRoute } from "@tanstack/react-router";
import { MemorySearch } from "@/features/agent/MemorySearch";

export const Route = createFileRoute("/_app/memory/search")({
  component: MemorySearch,
});
