import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/explore")({
  component: () => <div className="p-6">Explore (placeholder)</div>,
});
