import { createFileRoute } from "@tanstack/react-router";
import { KymaGraph } from "@kyma-ai/react/graph";

/**
 * Graph route — renders the embeddable KymaGraph component across all databases.
 *
 * URL search params:
 *   ?graph=<db/graphname>  — one-way: seeds focusQuery on first render so the
 *                            user lands on a specific namespace. Two-way URL sync
 *                            of selection state is not implemented (the package
 *                            store manages selection internally).
 *
 * Dropped behaviours vs. the old features/graph GraphView:
 *   - Two-way URL sync of selectedNodeId/graph filter: the KymaGraph store owns
 *     selection internally; we only seed focusQuery from the URL, not the reverse.
 *   - Direct access to overview/layout toggle state: managed inside KymaGraph.
 */

type GraphSearch = {
  graph?: string;
};

export const Route = createFileRoute("/_app/graph")({
  validateSearch: (search: Record<string, unknown>): GraphSearch => ({
    graph: typeof search.graph === "string" ? search.graph : undefined,
  }),
  component: GraphPage,
});

function GraphPage() {
  const { graph } = Route.useSearch();
  return (
    <div className="flex h-full w-full">
      <KymaGraph
        discover="all-databases"
        height="100%"
        focusQuery={graph}
      />
    </div>
  );
}
