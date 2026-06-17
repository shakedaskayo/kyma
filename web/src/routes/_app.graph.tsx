import { useCallback, useState } from "react";
import { createFileRoute } from "@tanstack/react-router";
import { KymaGraph } from "@kyma-ai/react/graph";
import { useConsumerStream } from "@/features/graph/useConsumerStream";

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
  /** Deep-link: node id to center + highlight (e.g. from a search result). */
  focus?: string;
};

export const Route = createFileRoute("/_app/graph")({
  validateSearch: (search: Record<string, unknown>): GraphSearch => ({
    graph: typeof search.graph === "string" ? search.graph : undefined,
    focus: typeof search.focus === "string" ? search.focus : undefined,
  }),
  component: GraphPage,
});

function GraphPage() {
  const { graph, focus } = Route.useSearch();
  // The in-graph "Live" checkbox drives this; OFF tears down the WebSocket.
  const [liveEnabled, setLiveEnabled] = useState(true);
  const liveConsumers = useConsumerStream({ enabled: liveEnabled });
  const onLiveConsumersToggle = useCallback((enabled: boolean) => setLiveEnabled(enabled), []);
  return (
    <div className="flex h-full w-full">
      <KymaGraph
        discover="all-databases"
        height="100%"
        focusQuery={graph}
        focusNodeId={focus}
        liveConsumers={liveConsumers}
        onLiveConsumersToggle={onLiveConsumersToggle}
      />
    </div>
  );
}
