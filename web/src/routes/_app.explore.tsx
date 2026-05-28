import { createFileRoute, redirect } from "@tanstack/react-router";

// Legacy `/explore` path → renamed to `/query`. Preserve the optional `q`
// param so deep links keep working.
export const Route = createFileRoute("/_app/explore")({
  validateSearch: (s: Record<string, unknown>) => ({ q: typeof s.q === "string" ? s.q : undefined }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/query", search: { q: search.q } });
  },
  component: () => null,
});
