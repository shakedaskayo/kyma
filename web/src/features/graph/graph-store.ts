import { create } from "zustand";
import type { LayoutAlgorithm } from "@/sdk/graph-layout";

type GraphStore = {
  /**
   * Selected namespace: "all" = unified union of every graph across every
   * database, or a composite `${database}/${graph}` key to focus a single
   * graph. The composite form disambiguates same-named graphs that exist in
   * different DBs (e.g. every DB has a `schema` graph).
   */
  graph: string;
  layout: LayoutAlgorithm;
  /**
   * Composite id `${namespace}::${id}` of the focused node. Node ids collide
   * across databases, so selection is namespace-qualified.
   */
  selectedNodeId: string | null;
  hoveredNodeId: string | null;
  labelFilter: string | null;
  relTypeFilter: string | null;
  /** Node-type labels hidden from the canvas (e.g. hide CodeFunction on big graphs). */
  hiddenLabels: string[];
  /** Composite `${database}/${graph}` keys hidden from the unified canvas. */
  hiddenNamespaces: string[];
  showEdgeLabels: boolean;
  /** Free-text query for the search box. Empty string = no search. */
  searchQuery: string;
  /** Whether the MiniMap overlay is rendered (auto-hidden on big graphs). */
  showMiniMap: boolean;
  setGraph(name: string): void;
  setLayout(layout: LayoutAlgorithm): void;
  selectNode(id: string | null): void;
  hoverNode(id: string | null): void;
  setLabelFilter(label: string | null): void;
  setRelTypeFilter(rel: string | null): void;
  toggleHiddenLabel(label: string): void;
  setHiddenLabels(labels: string[]): void;
  toggleHiddenNamespace(ns: string): void;
  setHiddenNamespaces(list: string[]): void;
  toggleEdgeLabels(): void;
  setSearchQuery(q: string): void;
  setShowMiniMap(v: boolean): void;
  reset(): void;
};

const initial = {
  graph: "all",
  layout: "force" as LayoutAlgorithm,
  selectedNodeId: null,
  hoveredNodeId: null,
  labelFilter: null,
  relTypeFilter: null,
  hiddenLabels: [] as string[],
  hiddenNamespaces: [] as string[],
  showEdgeLabels: false,
  searchQuery: "",
  showMiniMap: true,
};

export const useGraphStore = create<GraphStore>()((set) => ({
  ...initial,
  // Switching the focused namespace clears drill-down state but not which
  // namespaces are hidden (that's managed by the view when the graph set loads).
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
  toggleHiddenNamespace: (ns) =>
    set((s) => ({
      hiddenNamespaces: s.hiddenNamespaces.includes(ns)
        ? s.hiddenNamespaces.filter((n) => n !== ns)
        : [...s.hiddenNamespaces, ns],
    })),
  setHiddenNamespaces: (list) => set({ hiddenNamespaces: list }),
  toggleEdgeLabels: () => set((s) => ({ showEdgeLabels: !s.showEdgeLabels })),
  setSearchQuery: (q) => set({ searchQuery: q }),
  setShowMiniMap: (v) => set({ showMiniMap: v }),
  reset: () => set({ ...initial }),
}));
