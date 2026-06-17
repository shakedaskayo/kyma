/**
 * @kyma-ai/react/graph — embeddable Kyma graph view.
 *
 * Subpath entry so consumers who don't render the graph never pull
 * react-force-graph-2d into their bundle.
 */
export { KymaGraph, type KymaGraphProps } from "./KymaGraph";
export { GraphView, type GraphViewProps } from "./GraphView";
export {
  createGraphStore,
  GraphStoreContext,
  useGraphStore,
  type GraphStore,
  type GraphStoreState,
} from "./graph-store";
export { SigmaCanvas, type SigmaCanvasProps } from "./SigmaCanvas";
export { HullsLayer } from "./HullsLayer";
export { ConsumerBeamsLayer } from "./ConsumerBeamsLayer";
export { ConsumerDock } from "./ConsumerDock";
export {
  resolveConsumerKind,
  consumerIcon,
  consumerColor,
} from "./consumer-style";
export type {
  ConsumerKind,
  ConsumerVerb,
  ConsumerStatus,
  LiveConsumer,
  ConsumerBeam,
  LiveConsumersData,
} from "./consumer-types";
export type {
  GraphNode,
  GraphRelationship,
  GraphPayload,
  GraphStats,
  SearchHits,
} from "@kyma-ai/client";
