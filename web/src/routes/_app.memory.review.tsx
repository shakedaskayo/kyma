import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout route for /memory/review — the inbox lives in the index child.
// `source_run_id` deep-links the inbox to one dreaming run's candidates.
export const Route = createFileRoute("/_app/memory/review")({
  validateSearch: (s: Record<string, unknown>): { source_run_id?: string } => ({
    source_run_id: typeof s.source_run_id === "string" ? s.source_run_id : undefined,
  }),
  component: () => <Outlet />,
});
