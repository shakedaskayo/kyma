import { create } from "zustand";
import type { LayoutAlgorithm } from "@/sdk/graph-layout";

type GraphStore = {
  graph: string;
  layout: LayoutAlgorithm;
  selectedNodeId: string | null;
  hoveredNodeId: string | null;
  labelFilter: string | null;
  setGraph(name: string): void;
  setLayout(layout: LayoutAlgorithm): void;
  selectNode(id: string | null): void;
  hoverNode(id: string | null): void;
  setLabelFilter(label: string | null): void;
  reset(): void;
};

const initial = {
  graph: "schema",
  layout: "force" as LayoutAlgorithm,
  selectedNodeId: null,
  hoveredNodeId: null,
  labelFilter: null,
};

export const useGraphStore = create<GraphStore>()((set) => ({
  ...initial,
  setGraph: (name) => set({ graph: name, selectedNodeId: null, labelFilter: null }),
  setLayout: (layout) => set({ layout }),
  selectNode: (id) => set({ selectedNodeId: id }),
  hoverNode: (id) => set({ hoveredNodeId: id }),
  setLabelFilter: (label) => set({ labelFilter: label }),
  reset: () => set({ ...initial }),
}));
