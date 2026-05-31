import { createFileRoute, Outlet } from "@tanstack/react-router";

// Layout route for /connectors. The list lives in `_app.connectors.index.tsx`
// and the detail in `_app.connectors.$id.tsx`; both render through this Outlet.
// Without the Outlet, navigating to /connectors/:id would render this route's
// component and the child (detail) would never appear.
export const Route = createFileRoute("/_app/connectors")({
  component: () => <Outlet />,
});
