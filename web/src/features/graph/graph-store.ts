import { create } from "zustand";
import type { LayoutAlgorithm } from "@/sdk/graph-layout";

type GraphStore = {
  graph: string;
  layout: LayoutAlgorithm;
  selectedNodeId: string | null;
  hoveredNodeId: string | null;
  labelFilter: string | null;
  relTypeFilter: string | null;
  /** Node-type labels hidden from the canvas (e.g. hide CodeFunction on big graphs). */
  hiddenLabels: string[];
  showEdgeLabels: boolean;
  setGraph(name: string): void;
  setLayout(layout: LayoutAlgorithm): void;
  selectNode(id: string | null): void;
  hoverNode(id: string | null): void;
  setLabelFilter(label: string | null): void;
  setRelTypeFilter(rel: string | null): void;
  toggleHiddenLabel(label: string): void;
  setHiddenLabels(labels: string[]): void;
  toggleEdgeLabels(): void;
  reset(): void;
};

const initial = {
  graph: "schema",
  layout: "force" as LayoutAlgorithm,
  selectedNodeId: null,
  hoveredNodeId: null,
  labelFilter: null,
  relTypeFilter: null,
  hiddenLabels: [] as string[],
  showEdgeLabels: true,
};

export const useGraphStore = create<GraphStore>()((set) => ({
  ...initial,
  setGraph: (name) =>
    set({ graph: name, selectedNodeId: null, labelFilter: null, relTypeFilter: null, hiddenLabels: [] }),
  setLayout: (layout) => set({ layout }),
  selectNode: (id) => set({ selectedNodeId: id }),
  hoverNode: (id) => set({ hoveredNodeId: id }),
  setLabelFilter: (label) => set({ labelFilter: label, relTypeFilter: null }),
  setRelTypeFilter: (rel) => set({ relTypeFilter: rel, labelFilter: null }),
  toggleHiddenLabel: (label) =>
    set((s) => ({
      hiddenLabels: s.hiddenLabels.includes(label)
        ? s.hiddenLabels.filter((l) => l !== label)
        : [...s.hiddenLabels, label],
    })),
  setHiddenLabels: (labels) => set({ hiddenLabels: labels }),
  toggleEdgeLabels: () => set((s) => ({ showEdgeLabels: !s.showEdgeLabels })),
  reset: () => set({ ...initial }),
}));
