import { createFileRoute, redirect } from "@tanstack/react-router";

// Query Editor merged into the unified `/explore` page. Preserve the optional
// `q` deep-link param on redirect.
export const Route = createFileRoute("/_app/query")({
  validateSearch: (s: Record<string, unknown>) => ({ q: typeof s.q === "string" ? s.q : undefined }),
  beforeLoad: ({ search }) => {
    throw redirect({ to: "/explore", search: { q: search.q } });
  },
  component: () => null,
});
