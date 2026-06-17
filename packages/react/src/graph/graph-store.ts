/**
 * Per-instance graph store — created fresh for each mounted KymaGraph.
 *
 * Uses `createStore` from `zustand/vanilla` (NOT the module-level `create`)
 * so two mounted KymaGraph instances never share selection/layout state.
 * The store is provided via GraphStoreContext inside KymaGraph and consumed
 * via `useGraphStore`.
 */
import { createStore } from "zustand/vanilla";
import { useStore } from "zustand";
import { createContext, useContext } from "react";
import type { LayoutAlgorithm } from "@kyma-ai/client";
import type {
  ConsumerBeam,
  ConsumerStatus,
  LiveConsumer,
  LiveConsumersData,
} from "./consumer-types";

// ── State shape ───────────────────────────────────────────────────────────────

export type GraphStoreState = {
  /**
   * Selected namespace: "all" = unified union of every graph across every
   * database, or a composite `${database}/${graph}` key to focus a single
   * graph.
   */
  graph: string;
  layout: LayoutAlgorithm;
  /**
   * Composite id `${namespace}::${id}` of the focused node. Node ids collide
   * across databases, so selection is namespace-qualified.
   */
  selectedNodeId: string | null;
  hoveredNodeId: string | null;
  /** Bumped each time a deep-link focus is requested, so the canvas re-centers
   *  + zooms even when the same node is already selected. */
  focusSeq: number;
  labelFilter: string | null;
  relTypeFilter: string | null;
  /** Node-type labels hidden from the canvas. */
  hiddenLabels: string[];
  /** Composite `${database}/${graph}` keys hidden from the unified canvas. */
  hiddenNamespaces: string[];
  showEdgeLabels: boolean;
  /** Free-text query for the search box. Empty string = no search. */
  searchQuery: string;
  /** Whether the MiniMap overlay is rendered. */
  showMiniMap: boolean;

  // ── visual style toggles (WebGL renderer) ────────────────────────────────
  sizeByDegree: boolean;
  glow: boolean;
  animatedFlow: boolean;
  curvedEdges: boolean;
  communityHulls: boolean;
  /**
   * Overview mode: on large graphs, render only landmark nodes
   * (+ expanded / focused neighbourhoods) instead of the full hairball.
   */
  overview: boolean;

  /** Visited-node breadcrumb trail (composite ids, newest last, max 20). */
  trail: string[];
  /** Focus-neighborhood isolation root (double-click). Null = off. */
  focusModeId: string | null;
  commandBarOpen: boolean;

  // ── Live consumers overlay ───────────────────────────────────────────────
  /** Master toggle (the "Live" checkbox). Default on. */
  showLiveConsumers: boolean;
  /** Agents currently reading/writing memory (host-streamed snapshot). */
  liveConsumers: LiveConsumer[];
  /** In-flight beams the canvas overlay animates. */
  consumerBeams: ConsumerBeam[];
  consumerStatus: ConsumerStatus;
  consumerEventsPerMin: number;
  /** Dock row hovered → highlight its touched node on the canvas. */
  hoveredConsumerId: string | null;
  /** Dock row pinned → keep its beams alive ("watch this agent"). */
  pinnedConsumerId: string | null;
  /** Live consumers dock collapsed to a thin rail. */
  consumerDockCollapsed: boolean;
  /** Dock row expanded to show connection/process detail. */
  expandedConsumerId: string | null;

  pushTrail(id: string): void;
  jumpTrail(index: number): void;
  clearTrail(): void;
  setFocusMode(id: string | null): void;
  setCommandBarOpen(v: boolean): void;

  setGraph(name: string): void;
  setLayout(layout: LayoutAlgorithm): void;
  selectNode(id: string | null): void;
  /** Select a node AND request the canvas to center + zoom to it (deep-link focus). */
  focusNode(id: string | null): void;
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
  toggleSizeByDegree(): void;
  toggleGlow(): void;
  toggleAnimatedFlow(): void;
  toggleCurvedEdges(): void;
  toggleCommunityHulls(): void;
  setOverview(v: boolean): void;
  toggleLiveConsumers(): void;
  /** Ingest a fresh host snapshot (consumers + beams + status + rate). */
  setLiveConsumers(data: LiveConsumersData): void;
  hoverConsumer(id: string | null): void;
  pinConsumer(id: string | null): void;
  toggleConsumerDock(): void;
  expandConsumer(id: string | null): void;
  reset(): void;
};

// ── Initial state ─────────────────────────────────────────────────────────────

export const initialGraphState = {
  graph: "all",
  layout: "force" as LayoutAlgorithm,
  selectedNodeId: null,
  hoveredNodeId: null,
  focusSeq: 0,
  labelFilter: null,
  relTypeFilter: null,
  hiddenLabels: [] as string[],
  hiddenNamespaces: [] as string[],
  showEdgeLabels: false,
  searchQuery: "",
  showMiniMap: true,
  sizeByDegree: true,
  glow: true,
  animatedFlow: true,
  curvedEdges: true,
  communityHulls: true,
  overview: true,
  trail: [] as string[],
  focusModeId: null as string | null,
  commandBarOpen: false,
  showLiveConsumers: true,
  liveConsumers: [] as LiveConsumer[],
  consumerBeams: [] as ConsumerBeam[],
  consumerStatus: "idle" as ConsumerStatus,
  consumerEventsPerMin: 0,
  hoveredConsumerId: null as string | null,
  pinnedConsumerId: null as string | null,
  consumerDockCollapsed: false,
  expandedConsumerId: null as string | null,
};

// ── Factory ───────────────────────────────────────────────────────────────────

export type GraphStore = ReturnType<typeof createGraphStore>;

/**
 * Create a fresh, independent graph store. Each KymaGraph instance calls this
 * once (via useRef) so they never share state.
 */
export function createGraphStore(overrides?: Partial<typeof initialGraphState>) {
  const initial = { ...initialGraphState, ...overrides };
  return createStore<GraphStoreState>()((set) => ({
    ...initial,
    pushTrail: (id) =>
      set((s) => {
        if (s.trail[s.trail.length - 1] === id) return {};
        return { trail: [...s.trail, id].slice(-20) };
      }),
    jumpTrail: (index) =>
      set((s) => {
        const target = s.trail[index];
        if (!target) return {};
        return {
          trail: s.trail.slice(0, index + 1),
          selectedNodeId: target,
          focusSeq: s.focusSeq + 1,
        };
      }),
    clearTrail: () => set({ trail: [] }),
    setFocusMode: (id) => set({ focusModeId: id }),
    setCommandBarOpen: (v) => set({ commandBarOpen: v }),
    setGraph: (name) =>
      set({ graph: name, selectedNodeId: null, labelFilter: null, relTypeFilter: null, hiddenLabels: [], focusModeId: null }),
    setLayout: (layout) => set({ layout }),
    selectNode: (id) => set({ selectedNodeId: id }),
    focusNode: (id) => set((s) => ({ selectedNodeId: id, focusSeq: s.focusSeq + 1 })),
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
    toggleSizeByDegree: () => set((s) => ({ sizeByDegree: !s.sizeByDegree })),
    toggleGlow: () => set((s) => ({ glow: !s.glow })),
    toggleAnimatedFlow: () => set((s) => ({ animatedFlow: !s.animatedFlow })),
    toggleCurvedEdges: () => set((s) => ({ curvedEdges: !s.curvedEdges })),
    toggleCommunityHulls: () => set((s) => ({ communityHulls: !s.communityHulls })),
    setOverview: (v) => set({ overview: v }),
    toggleLiveConsumers: () => set((s) => ({ showLiveConsumers: !s.showLiveConsumers })),
    setLiveConsumers: (data) =>
      set({
        liveConsumers: data.consumers,
        consumerBeams: data.beams,
        consumerStatus: data.status,
        consumerEventsPerMin: data.eventsPerMin,
      }),
    hoverConsumer: (id) => set({ hoveredConsumerId: id }),
    pinConsumer: (id) =>
      set((s) => ({ pinnedConsumerId: s.pinnedConsumerId === id ? null : id })),
    toggleConsumerDock: () => set((s) => ({ consumerDockCollapsed: !s.consumerDockCollapsed })),
    expandConsumer: (id) =>
      set((s) => ({ expandedConsumerId: s.expandedConsumerId === id ? null : id })),
    reset: () => set({ ...initial }),
  }));
}

// ── Context ───────────────────────────────────────────────────────────────────

export const GraphStoreContext = createContext<GraphStore | null>(null);

/**
 * Hook to read from the nearest GraphStoreContext. Throws if used outside a
 * GraphStoreProvider (i.e. outside a KymaGraph).
 */
export function useGraphStore<T>(selector: (s: GraphStoreState) => T): T {
  const store = useContext(GraphStoreContext);
  if (!store) {
    throw new Error("useGraphStore must be used inside <KymaGraph>");
  }
  return useStore(store, selector);
}
