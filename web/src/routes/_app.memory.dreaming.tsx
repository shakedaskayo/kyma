import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout route for /memory/dreaming — the panel lives in the index child and
// the run drilldown in `$runId`; both render through this Outlet.
export const Route = createFileRoute("/_app/memory/dreaming")({
  component: () => <Outlet />,
});
