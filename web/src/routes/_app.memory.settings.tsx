import { createFileRoute } from "@tanstack/react-router";
import { MemorySettingsPanel } from "@/features/agent/MemorySettingsPanel";

export const Route = createFileRoute("/_app/memory/settings")({
  component: MemorySettingsPanel,
});
