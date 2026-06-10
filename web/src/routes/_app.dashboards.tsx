import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout route for /dashboards. The list lives in `_app.dashboards.index.tsx`
// and the detail in `_app.dashboards.$id.tsx`; both render through this Outlet.
// Without the Outlet, navigating to /dashboards/:id would render this route's
// component and the child (detail) would never appear.
export const Route = createFileRoute("/_app/dashboards")({
  component: () => <Outlet />,
});
