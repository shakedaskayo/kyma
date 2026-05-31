import { createContext } from "react";

/**
 * Structural canvas data needed by individual node components to decide their
 * own dim/highlight state. Recomputed only when the graph structure changes —
 * NOT on hover. Each `GraphNodeView` subscribes to selection/hover state from
 * the graph store directly, so a hover only re-renders the nodes whose
 * boolean state actually flips, not the whole canvas.
 */
export interface CanvasContextValue {
  /** Composite-keyed adjacency for the highlight-neighbours behaviour. */
  neighborsByCompositeId: Map<string, Set<string>>;
  /** For each relationship type, the set of composite-keyed node ids touched. */
  nodesByRelType: Map<string, Set<string>>;
}

export const CanvasContext = createContext<CanvasContextValue>({
  neighborsByCompositeId: new Map(),
  nodesByRelType: new Map(),
});
