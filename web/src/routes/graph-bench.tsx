/**
 * /graph-bench — Synthetic graph renderer benchmark.
 *
 * Standalone route (no auth / no server dependency). Generates a deterministic
 * synthetic graph in-memory using a mulberry32 PRNG, drives SigmaCanvas directly,
 * and shows a live FPS meter + node/edge count chip.
 *
 * Usage:  /graph-bench?n=50000&e=100000
 * Defaults: n=50000 nodes, e=100000 edges.
 */

import { createFileRoute } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  SigmaCanvas,
  createGraphStore,
  GraphStoreContext,
} from "@pensieve-ai/react/graph";
import { PensieveProvider, pensieveDark } from "@pensieve-ai/react";
import type { GraphNode, GraphRelationship } from "@pensieve-ai/react/graph";
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Sigma = any;

// ── Route definition ────────────────────────────────────────────────────────

type BenchSearch = {
  n: number;
  e: number;
};

/** Parse a URL search param to a positive integer, falling back to `def`. */
function parseIntParam(v: unknown, def: number): number {
  if (typeof v === "number") return Math.max(1, v);
  if (typeof v === "string") {
    const n = parseInt(v, 10);
    return Number.isFinite(n) && n > 0 ? n : def;
  }
  return def;
}

export const Route = createFileRoute("/graph-bench")({
  validateSearch: (search: Record<string, unknown>): BenchSearch => ({
    n: parseIntParam(search.n, 50_000),
    e: parseIntParam(search.e, 100_000),
  }),
  component: GraphBenchPage,
});

// ── Mulberry32 PRNG ─────────────────────────────────────────────────────────

/** Deterministic mulberry32 PRNG seeded with a constant. */
function makePrng(seed: number): () => number {
  let s = seed >>> 0;
  return function next(): number {
    s += 0x6d2b79f5;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ── Synthetic data generator ─────────────────────────────────────────────────

const BENCH_NS = "bench/synthetic";
const BENCH_DB = "bench";
const SEED = 0xdeadbeef;

const LABEL_GROUPS = [
  "Person",
  "Organization",
  "Project",
  "Document",
  "Event",
  "Location",
  "Product",
  "Service",
  "Technology",
  "Concept",
  "Dataset",
  "Pipeline",
] as const;

const REL_TYPES = [
  "KNOWS",
  "WORKS_FOR",
  "CREATED",
  "DEPENDS_ON",
  "BELONGS_TO",
  "REFERENCES",
  "PART_OF",
] as const;

/** Layout: 12 groups in a 4000×2500 box, each cluster centered on a grid point. */
const GROUP_CENTERS: Array<{ x: number; y: number }> = LABEL_GROUPS.map(
  (_, i) => {
    const cols = 4;
    const col = i % cols;
    const row = Math.floor(i / cols);
    return {
      x: 400 + col * 1066,
      y: 300 + row * 640,
    };
  },
);

const CLUSTER_SPREAD = 320; // ± pixels within each group cluster

interface SyntheticGraph {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  positions: Map<string, { x: number; y: number }>;
  activeNamespaces: Set<string>;
}

function generateSyntheticGraph(n: number, e: number): SyntheticGraph {
  const rng = makePrng(SEED);

  // Generate nodes
  const nodes: GraphNode[] = [];
  const positions = new Map<string, { x: number; y: number }>();

  // Track hub nodes per group (top 5% = hubs for biased edge assignment)
  const groupNodes: string[][] = LABEL_GROUPS.map(() => []);

  for (let i = 0; i < n; i++) {
    const groupIdx = Math.floor(rng() * LABEL_GROUPS.length);
    const label = LABEL_GROUPS[groupIdx];
    const center = GROUP_CENTERS[groupIdx];

    // Gaussian-ish spread: sum of two uniforms → triangle distribution
    const dx = (rng() + rng() - 1) * CLUSTER_SPREAD;
    const dy = (rng() + rng() - 1) * CLUSTER_SPREAD;

    const id = `n${i}`;
    const compositeId = `${BENCH_NS}::${id}`;

    nodes.push({
      id,
      labels: [label],
      properties: { name: `${label} ${i}` },
      metadata: {
        created_at: "2026-01-01T00:00:00Z",
        updated_at: "2026-01-01T00:00:00Z",
        realm: "bench",
      },
      namespace: BENCH_NS,
      database: BENCH_DB,
    });

    positions.set(compositeId, {
      x: center.x + dx,
      y: center.y + dy,
    });

    groupNodes[groupIdx].push(id);
  }

  // Designate hub nodes per group: first 5% of each group's nodes
  const hubsPerGroup = groupNodes.map((g) =>
    g.slice(0, Math.max(1, Math.ceil(g.length * 0.05))),
  );

  // Generate edges — 70% intra-group (hub-spoke), 30% inter-group
  const edges: GraphRelationship[] = [];
  const edgeSet = new Set<string>();

  for (let i = 0; i < e; i++) {
    const relType = REL_TYPES[Math.floor(rng() * REL_TYPES.length)];

    let srcId: string;
    let tgtId: string;
    let attempts = 0;

    do {
      const isIntraGroup = rng() < 0.7;

      if (isIntraGroup) {
        // Hub-spoke: source from hubs, target from full group
        const gIdx = Math.floor(rng() * LABEL_GROUPS.length);
        const hubs = hubsPerGroup[gIdx];
        const group = groupNodes[gIdx];
        if (hubs.length === 0 || group.length < 2) {
          srcId = nodes[Math.floor(rng() * nodes.length)].id;
          tgtId = nodes[Math.floor(rng() * nodes.length)].id;
        } else {
          srcId = hubs[Math.floor(rng() * hubs.length)];
          tgtId = group[Math.floor(rng() * group.length)];
        }
      } else {
        // Cross-group
        const gSrc = Math.floor(rng() * LABEL_GROUPS.length);
        const gTgt = Math.floor(rng() * LABEL_GROUPS.length);
        const srcGroup = groupNodes[gSrc];
        const tgtGroup = groupNodes[gTgt];
        srcId = srcGroup.length
          ? srcGroup[Math.floor(rng() * srcGroup.length)]
          : `n0`;
        tgtId = tgtGroup.length
          ? tgtGroup[Math.floor(rng() * tgtGroup.length)]
          : `n0`;
      }

      attempts++;
    } while (srcId === tgtId && attempts < 5);

    if (srcId === tgtId) continue;

    const edgeKey = `${srcId}->${tgtId}:${relType}`;
    if (edgeSet.has(edgeKey)) continue;
    edgeSet.add(edgeKey);

    edges.push({
      id: `e${i}`,
      source_id: srcId,
      target_id: tgtId,
      relationship_type: relType,
      properties: {},
      namespace: BENCH_NS,
      database: BENCH_DB,
    });
  }

  return {
    nodes,
    edges,
    positions,
    activeNamespaces: new Set([BENCH_NS]),
  };
}

// ── FPS meter ────────────────────────────────────────────────────────────────

function useFpsMeter(): number {
  const [fps, setFps] = useState(0);
  const frameCount = useRef(0);
  const lastTime = useRef(performance.now());

  useEffect(() => {
    let rafId: number;

    function tick() {
      frameCount.current++;
      const now = performance.now();
      const elapsed = now - lastTime.current;
      if (elapsed >= 1000) {
        setFps(Math.round((frameCount.current * 1000) / elapsed));
        frameCount.current = 0;
        lastTime.current = now;
      }
      rafId = requestAnimationFrame(tick);
    }

    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, []);

  return fps;
}

// ── Page component ────────────────────────────────────────────────────────────

function GraphBenchPage() {
  const { n, e } = Route.useSearch();
  const fps = useFpsMeter();

  // One store per mount — createGraphStore with no overrides
  const store = useMemo(() => createGraphStore(), []);

  // Generate synthetic graph only when n/e change
  const { nodes, edges, positions, activeNamespaces } = useMemo(
    () => generateSyntheticGraph(n, e),
    [n, e],
  );

  // Sigma instance ref for potential future interactions
  const sigmaRef = useRef<Sigma | null>(null);
  const handleSigmaReady = useCallback((sigma: Sigma) => {
    sigmaRef.current = sigma;
  }, []);

  // No-op handlers for SigmaCanvas
  const noop = useCallback(() => {}, []);
  const noopNull = useCallback((_: string | null) => {}, []);

  return (
    <PensieveProvider
      endpoint="http://localhost:0"
      auth={{ getToken: async () => "" }}
      theme={pensieveDark}
    >
      <GraphStoreContext.Provider value={store}>
        <div
          style={{
            position: "fixed",
            inset: 0,
            background: "#0b1120",
            overflow: "hidden",
          }}
        >
          {/* Sigma canvas fills the whole viewport */}
          <SigmaCanvas
            nodes={nodes}
            edges={edges}
            positions={positions}
            version={1}
            activeNamespaces={activeNamespaces}
            onNodeClick={noop}
            onNodeHover={noopNull}
            onNodeDoubleClick={noop}
            onSigmaReady={handleSigmaReady}
          />

          {/* Overlay chips */}
          <div
            style={{
              position: "absolute",
              top: 12,
              right: 12,
              display: "flex",
              flexDirection: "column",
              gap: 6,
              alignItems: "flex-end",
              pointerEvents: "none",
              fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
              fontSize: 12,
            }}
          >
            {/* FPS chip */}
            <div
              style={{
                background: "rgba(0,0,0,0.65)",
                border: "1px solid rgba(148,163,184,0.25)",
                borderRadius: 6,
                padding: "4px 10px",
                color: fps >= 50 ? "#34d399" : fps >= 30 ? "#fbbf24" : "#f87171",
                minWidth: 72,
                textAlign: "right",
              }}
            >
              {fps} fps
            </div>

            {/* Count chip */}
            <div
              style={{
                background: "rgba(0,0,0,0.65)",
                border: "1px solid rgba(148,163,184,0.25)",
                borderRadius: 6,
                padding: "4px 10px",
                color: "#94a3b8",
              }}
            >
              {nodes.length.toLocaleString()} nodes
              {" · "}
              {edges.length.toLocaleString()} edges
            </div>

            {/* Label */}
            <div
              style={{
                background: "rgba(0,0,0,0.65)",
                border: "1px solid rgba(148,163,184,0.15)",
                borderRadius: 6,
                padding: "4px 10px",
                color: "#475569",
                fontSize: 10,
              }}
            >
              bench/synthetic · mulberry32 PRNG
            </div>
          </div>
        </div>
      </GraphStoreContext.Provider>
    </PensieveProvider>
  );
}
