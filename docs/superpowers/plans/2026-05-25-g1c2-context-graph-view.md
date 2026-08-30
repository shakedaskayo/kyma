# Graph Layer G1c.2 — Context Graph View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make the Context Graph *visible* in the pensieve web app — an `@xyflow/react` canvas rendering the schema-graph from G1c.1's data layer, with a custom node, a node-detail panel, a legend, a layout toolbar, the `GraphView` orchestrator, a `/graph` route, and a shell nav link.

**Architecture:** A new `features/graph/` view tier on top of G1c.1. `GraphView` loads the overview via `useGraphOverview`, holds the working node/edge set in local state (merging neighbor-expansion results), reads UI state from `useGraphStore`, and composes `GraphCanvas` + `NodeDetailPanel` + `GraphLegend` + `CanvasToolbar`. The canvas, custom node, and panels are ported from the agentcy project; the orchestrator, route, nav, and `badge` primitive are pensieve-specific and given in full.

**Tech Stack:** React 18, `@xyflow/react` (new dep, added via **pnpm** — web is a pnpm workspace package; never npm), TanStack Router (file-based), Tailwind tokens, lucide-react.

**Reference:**
- Data layer (G1c.1): `web/src/sdk/graph.ts` (types + fetch), `web/src/sdk/graph-layout.ts` (`computeLayout`, `LayoutAlgorithm`, `getLabelColor`, `getRelationshipColor`), `web/src/features/graph/graph-store.ts` (`useGraphStore`), `web/src/features/graph/useGraph.ts` (hooks).
- Port sources (read these): `/Users/shakedaskayo/shaked/projects/agentcylabs/agentcy/frontend/components/graph/{graph-canvas,graph-node,node-detail-panel,graph-legend,canvas-toolbar}.tsx`.
- Web conventions: components in `web/src/components/ui/`, `cn` at `@/lib/utils`, route convention `web/src/routes/_app.<name>.tsx` → `createFileRoute("/_app/<name>")`, shell nav in `web/src/app/shell.tsx`, Tailwind tokens (`bg-background`, `text-foreground`, `text-muted-foreground`, `border-border`, `bg-accent`, `bg-muted`), dark mode class-based.

**Working dir:** worktree `/Users/shakedaskayo/shaked/projects/pensieve/.claude/worktrees/feature+graph-layer`, web under `web/`. Run `cd web && npx tsc --noEmit`, `cd web && npx vitest run`, `cd web && npm run build` (vite). **Use pnpm for deps.**

---

## Component contracts (the seams that hold the port together)

```ts
// All domain types imported from "@/sdk/graph".
import type { GraphNode, GraphRelationship, GraphStats } from "@/sdk/graph";
import type { LayoutAlgorithm } from "@/sdk/graph-layout";

// GraphCanvas
interface GraphCanvasProps {
  nodes: GraphNode[];
  edges: GraphRelationship[];
  layout: LayoutAlgorithm;
  selectedNodeId: string | null;
  hoveredNodeId: string | null;
  labelFilter: string | null;           // dims non-matching nodes when set
  onNodeClick: (id: string) => void;
  onNodeHover: (id: string | null) => void;
}

// Custom xyflow node data (registered as nodeType "graphNode")
interface GraphNodeData {
  label: string;        // display name (table name)
  labels: string[];     // ["Table"]
  color: string;        // getLabelColor(labels[0])
  isSelected: boolean;
  isDimmed: boolean;
  [key: string]: unknown;   // xyflow Node data index signature
}

// NodeDetailPanel
interface NodeDetailPanelProps {
  node: GraphNode;
  edges: GraphRelationship[];                 // current working edge set
  nodesById: Map<string, GraphNode>;          // for resolving connection names
  onClose: () => void;
  onExpand: (id: string) => void;             // fetch + merge neighbors
  onSelect: (id: string) => void;             // select a connected node
}

// GraphLegend
interface GraphLegendProps {
  stats: GraphStats | undefined;
  activeLabel: string | null;
  onLabelClick: (label: string | null) => void;
}

// CanvasToolbar
interface CanvasToolbarProps {
  layout: LayoutAlgorithm;
  onLayoutChange: (l: LayoutAlgorithm) => void;
}
```

---

## Task 1: dependency + `badge` primitive + custom node

**Files:** `web/package.json` (+ root `pnpm-lock.yaml`), `web/src/components/ui/badge.tsx`, `web/src/features/graph/GraphNodeView.tsx`.

- [ ] **Step 1: add `@xyflow/react` via pnpm.** From the worktree: `cd web && pnpm add @xyflow/react`. This updates `web/package.json` + the root `pnpm-lock.yaml`. Do NOT use npm. Verify it installed: `cd web && node -e "require.resolve('@xyflow/react')" && echo ok` (or check `web/node_modules/@xyflow`).

- [ ] **Step 2: add the `badge` primitive** — `web/src/components/ui/badge.tsx` (standard shadcn badge, matching the existing `button.tsx` CVA style in this repo):

```tsx
import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium transition-colors",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary/15 text-foreground",
        secondary: "border-transparent bg-muted text-muted-foreground",
        outline: "text-foreground",
      },
    },
    defaultVariants: { variant: "default" },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

export function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />;
}
```
NOTE: confirm `class-variance-authority` is a dependency (it is — `button.tsx` uses it). Confirm `cn` exists at `@/lib/utils`.

- [ ] **Step 3: custom node** — `web/src/features/graph/GraphNodeView.tsx`. PORT from agentcy `graph-node.tsx`, but SIMPLIFY: drop `framer-motion` (use a CSS transition), drop `SourceIcon`/provider-badge entirely (pensieve's schema-graph has no connector brands), drop the abbreviation logic if you like (showing the full short label is fine). Keep: 4 invisible `Handle`s (top/bottom/left/right), a colored circle (44px, 52px when selected) using the node's `color` with a radial-gradient/border like the original, the display label below, `opacity` driven by `isDimmed`. Use `memo`. Reference shape:

```tsx
import { memo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";

function GraphNodeViewInner({ data }: NodeProps) {
  const d = data as { label: string; labels: string[]; color: string; isSelected: boolean; isDimmed: boolean };
  const color = d.color;
  const size = d.isSelected ? 52 : 44;
  return (
    <div
      style={{ opacity: d.isDimmed ? 0.15 : 1, transition: "opacity 0.2s" }}
      className="flex flex-col items-center"
    >
      <Handle type="target" position={Position.Top} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Bottom} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="target" position={Position.Left} style={{ opacity: 0, width: 1, height: 1 }} />
      <Handle type="source" position={Position.Right} style={{ opacity: 0, width: 1, height: 1 }} />
      <div
        style={{
          width: size, height: size, borderRadius: "9999px",
          background: `radial-gradient(circle at 40% 35%, ${color}40, ${color}15)`,
          border: `1.5px solid ${color}`,
          boxShadow: d.isSelected ? `0 0 20px ${color}55` : "none",
          transition: "all 0.15s",
        }}
      />
      <div
        className={`mt-1 max-w-[120px] truncate text-[10px] ${d.isSelected ? "font-semibold text-foreground" : "text-muted-foreground"}`}
        title={d.label}
      >
        {d.label}
      </div>
    </div>
  );
}
export const GraphNodeView = memo(GraphNodeViewInner);
```

- [ ] **Step 4: verify build** — `cd web && npx tsc --noEmit` clean. Commit:
```bash
git add web/package.json pnpm-lock.yaml web/src/components/ui/badge.tsx web/src/features/graph/GraphNodeView.tsx
git commit -m "feat(web/graph): add @xyflow/react, badge primitive, custom graph node"
```
(If pnpm-lock.yaml is at the repo root, `git add ../pnpm-lock.yaml` from `web/`, or add it from the worktree root.)

---

## Task 2: `GraphCanvas.tsx`

**Files:** `web/src/features/graph/GraphCanvas.tsx`.

PORT from agentcy `graph-canvas.tsx`, adapting to the `GraphCanvasProps` contract above. Key requirements:

- [ ] **Step 1:** Implement `GraphCanvas` exporting a component wrapped in `<ReactFlowProvider>` (the agentcy file does this — inner component uses the `useReactFlow` hook for `fitView`). Import `import "@xyflow/react/dist/style.css";` at the top.
- [ ] **Step 2:** Register `nodeTypes = { graphNode: GraphNodeView }` (module-level constant, not re-created per render).
- [ ] **Step 3:** Build xyflow nodes/edges in a `useMemo` keyed on the node ids, edge ids, `layout`, `selectedNodeId`, `hoveredNodeId`, `labelFilter`:
  - Positions: `const pos = computeLayout(layout, nodes, edges, 1400, 900)` (import from `@/sdk/graph-layout`).
  - For each `GraphNode`: `{ id, type: "graphNode", position: pos.get(id) ?? {x:0,y:0}, data: { label: (n.properties.name as string) || n.id.split("::").pop() || n.id, labels: n.labels, color: getLabelColor(n.labels[0] ?? ""), isSelected: n.id === selectedNodeId, isDimmed: dim(n) } }`. `dim(n)` = `labelFilter ? !n.labels.includes(labelFilter) : (hl && !hl.has(n.id))` where `hl` is the highlight set (selected/hovered node + its neighbors), else false.
  - Edges: one xyflow edge per `GraphRelationship`: `{ id, source: r.source_id, target: r.target_id, type: "default", style: { stroke: getRelationshipColor(r.relationship_type), strokeWidth: highlighted ? 2.5 : 1.2, opacity: dimmed ? 0.1 : 0.7 }, markerEnd: { type: MarkerType.ArrowClosed, color } }`. (Skip the agentcy 5+-edge collapsing for v1 — the schema-graph is small.)
- [ ] **Step 4:** Highlight set: when `selectedNodeId` or `hoveredNodeId` is set, compute the set of that node + all directly connected node ids (walk `edges`); nodes/edges outside the set get dimmed styling. When `labelFilter` is set, dim nodes whose `labels` don't include it.
- [ ] **Step 5:** `<ReactFlow nodes edges nodeTypes onNodeClick={(_,n)=>onNodeClick(n.id)} onNodeMouseEnter={(_,n)=>onNodeHover(n.id)} onNodeMouseLeave={()=>onNodeHover(null)} onPaneClick={()=>onNodeClick("")} fitView minZoom={0.05} maxZoom={4} proOptions={{hideAttribution:true}}>` with `<Controls showInteractive={false} />`, `<MiniMap nodeColor={(n)=>(n.data as any).color ?? "#64748b"} />`, `<Background variant={BackgroundVariant.Dots} gap={20} size={0.6} />`. (`onPaneClick` passing `""` lets the orchestrator treat empty-string as deselect.)
- [ ] **Step 6:** When the node set changes, call `fitView({ padding: 0.2, maxZoom: 1.5 })` via a `useReactFlow` ref in a double-`requestAnimationFrame` (as agentcy does) so layout settles first.
- [ ] **Step 7:** `cd web && npx tsc --noEmit` clean. Commit:
```bash
git add web/src/features/graph/GraphCanvas.tsx
git commit -m "feat(web/graph): xyflow Context Graph canvas"
```

---

## Task 3: detail panel + legend + toolbar

**Files:** `web/src/features/graph/NodeDetailPanel.tsx`, `GraphLegend.tsx`, `CanvasToolbar.tsx`.

- [ ] **Step 1: `NodeDetailPanel.tsx`** — PORT from agentcy `node-detail-panel.tsx`, simplified to the `NodeDetailPanelProps` contract. NO Radix Tabs (this repo lacks them) — use two local-state buttons ("Details" / "Connections") toggling a `useState` tab. Use the `Badge` from Task 1, `ScrollArea` from `@/components/ui/scroll-area`, lucide icons (`X`, `ArrowRight`, `ArrowLeft`, `Copy`). Right-side panel `w-80 border-l bg-background`. Sections:
  - Header: node display name + label `Badge`s (colored via `getLabelColor`) + close `X`.
  - Details tab: `Object.entries(node.properties)` as click-to-copy rows (`navigator.clipboard.writeText`), truncate long values; metadata block (id, realm, source_type, updated_at).
  - Connections tab: derive from `edges` — incoming = edges where `target_id===node.id`, outgoing where `source_id===node.id`; group by `relationship_type`; each row shows a direction arrow + the other node's display name (resolve via `nodesById`), clicking calls `onSelect(otherId)`.
  - An "Expand neighbors" button calling `onExpand(node.id)`.
- [ ] **Step 2: `GraphLegend.tsx`** — PORT/simplify from agentcy `graph-legend.tsx`. Positioned `absolute right-3 top-3`. Reads `stats.label_counts`, renders a row per label: color dot (`getLabelColor`) + label + count; clicking toggles `onLabelClick(label)` (or `null` if it's already active). Active label row highlighted (`bg-primary/15`). Small `Card`-ish container (`rounded-md border bg-background/90 p-2 text-xs`).
- [ ] **Step 3: `CanvasToolbar.tsx`** — minimal: positioned `absolute bottom-3 left-1/2 -translate-x-1/2`, a small segmented control of 3 buttons (Force/Grid/Radial) calling `onLayoutChange`; active layout highlighted. (Zoom/fit come from xyflow `<Controls>` — no need to duplicate.)
- [ ] **Step 4:** `cd web && npx tsc --noEmit` clean. Commit:
```bash
git add web/src/features/graph/NodeDetailPanel.tsx web/src/features/graph/GraphLegend.tsx web/src/features/graph/CanvasToolbar.tsx
git commit -m "feat(web/graph): node-detail panel, legend, layout toolbar"
```

---

## Task 4: `GraphView` orchestrator + route + nav

**Files:** `web/src/features/graph/GraphView.tsx`, `web/src/routes/_app.graph.tsx`, modify `web/src/app/shell.tsx`.

- [ ] **Step 1: `GraphView.tsx`** (full code):

```tsx
import { useEffect, useMemo, useState } from "react";
import { Network } from "lucide-react";
import { useGraphStore } from "./graph-store";
import { useGraphOverview, useExpandNeighbors } from "./useGraph";
import { getNode, type GraphNode, type GraphRelationship } from "@/sdk/graph";
import { useSession } from "@/sdk/session";
import { GraphCanvas } from "./GraphCanvas";
import { NodeDetailPanel } from "./NodeDetailPanel";
import { GraphLegend } from "./GraphLegend";
import { CanvasToolbar } from "./CanvasToolbar";

export function GraphView() {
  const graph = useGraphStore((s) => s.graph);
  const layout = useGraphStore((s) => s.layout);
  const setLayout = useGraphStore((s) => s.setLayout);
  const selectedNodeId = useGraphStore((s) => s.selectedNodeId);
  const selectNode = useGraphStore((s) => s.selectNode);
  const hoveredNodeId = useGraphStore((s) => s.hoveredNodeId);
  const hoverNode = useGraphStore((s) => s.hoverNode);
  const labelFilter = useGraphStore((s) => s.labelFilter);
  const setLabelFilter = useGraphStore((s) => s.setLabelFilter);

  const overview = useGraphOverview(graph);
  const expand = useExpandNeighbors(graph);
  const { endpoint, token } = useSession();

  // Working set: seeded from the overview, augmented by neighbor expansion.
  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [edges, setEdges] = useState<GraphRelationship[]>([]);

  useEffect(() => {
    if (overview.data) {
      setNodes(overview.data.nodes);
      setEdges(overview.data.edges);
    }
  }, [overview.data]);

  const nodesById = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);

  async function handleExpand(id: string) {
    const exp = await expand([id], "both");
    // merge edges (dedupe by id)
    setEdges((prev) => {
      const seen = new Set(prev.map((e) => e.id));
      return [...prev, ...exp.edges.filter((e) => !seen.has(e.id))];
    });
    // fetch any genuinely-new nodes and merge
    const have = new Set(nodes.map((n) => n.id));
    const missing = exp.new_node_ids.filter((nid) => !have.has(nid));
    if (missing.length) {
      const fetched = await Promise.all(
        missing.map((nid) => getNode({ endpoint, token, graph, id: nid }).catch(() => null)),
      );
      const add = fetched.filter((n): n is GraphNode => n != null);
      if (add.length) {
        setNodes((prev) => {
          const seen = new Set(prev.map((n) => n.id));
          return [...prev, ...add.filter((n) => !seen.has(n.id))];
        });
      }
    }
  }

  const selectedNode = selectedNodeId ? nodesById.get(selectedNodeId) ?? null : null;

  return (
    <div className="relative flex h-full w-full">
      <div className="relative flex-1">
        {overview.isLoading && (
          <div className="absolute inset-0 z-10 flex items-center justify-center text-sm text-muted-foreground">
            Loading graph…
          </div>
        )}
        {overview.isError && (
          <div className="absolute inset-0 z-10 flex items-center justify-center text-sm text-destructive">
            Failed to load graph: {(overview.error as Error)?.message}
          </div>
        )}
        {!overview.isLoading && !overview.isError && nodes.length === 0 && (
          <div className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-2 text-muted-foreground">
            <Network className="h-8 w-8" />
            <div className="text-sm">No graph data yet — ingest some tables to see the schema graph.</div>
          </div>
        )}
        <GraphCanvas
          nodes={nodes}
          edges={edges}
          layout={layout}
          selectedNodeId={selectedNodeId}
          hoveredNodeId={hoveredNodeId}
          labelFilter={labelFilter}
          onNodeClick={(id) => selectNode(id === "" ? null : id)}
          onNodeHover={hoverNode}
        />
        <GraphLegend
          stats={overview.data?.stats}
          activeLabel={labelFilter}
          onLabelClick={(l) => setLabelFilter(l === labelFilter ? null : l)}
        />
        <CanvasToolbar layout={layout} onLayoutChange={setLayout} />
      </div>
      {selectedNode && (
        <NodeDetailPanel
          node={selectedNode}
          edges={edges}
          nodesById={nodesById}
          onClose={() => selectNode(null)}
          onExpand={handleExpand}
          onSelect={(id) => selectNode(id)}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 2: route** — `web/src/routes/_app.graph.tsx`:
```tsx
import { createFileRoute } from "@tanstack/react-router";
import { GraphView } from "@/features/graph/GraphView";

export const Route = createFileRoute("/_app/graph")({
  component: GraphView,
});
```
(The TanStackRouterVite plugin regenerates `routeTree.gen.ts` automatically when the dev server / build runs. If `npm run build` complains the route isn't in the generated tree, run the dev server once or `cd web && npx tsc` after `npm run dev` has regenerated it — confirm by checking `routeTree.gen.ts` includes `/_app/graph`. Do NOT hand-edit the generated file unless the plugin fails.)

- [ ] **Step 3: nav link** — in `web/src/app/shell.tsx`, add `Network` (or `Workflow`) to the `lucide-react` import, and add a `<Link>` after the "Ask Pensieve" link:
```tsx
<Link to="/graph" className={btn(active.startsWith("/graph"))}>
  <Network className="h-4 w-4" /> Graph
</Link>
```

- [ ] **Step 4: verify** — `cd web && npx tsc --noEmit` clean; `cd web && npm run build` succeeds (this also regenerates the route tree). Commit:
```bash
git add web/src/features/graph/GraphView.tsx web/src/routes/_app.graph.tsx web/src/app/shell.tsx web/src/app/routeTree.gen.ts
git commit -m "feat(web/graph): GraphView orchestrator + /graph route + nav link"
```

---

## Task 5: verify the view builds & lints

- [ ] **Step 1:** `cd web && npx tsc --noEmit` → clean.
- [ ] **Step 2:** `cd web && npm run build` → succeeds (vite build; confirms the route tree + all imports resolve + xyflow bundles).
- [ ] **Step 3:** `cd web && npx vitest run` → the G1c.1 data-layer tests still pass (no rendering tests added here; the view is verified by build + the G1d manual run).
- [ ] **Step 4:** `cd web && npm run lint` (if configured) → no new errors in `features/graph/**`.

---

## Self-review notes (for the executor)

- **Port fidelity:** the canvas/node/panels are ports — read the agentcy source, but conform to the `GraphCanvasProps`/`NodeDetailPanelProps`/etc. contracts above (those are the integration seams). Drop agentcy-only deps (`framer-motion`, `SourceIcon`, `@/lib/api/client`, `next/navigation`, Radix `Tabs`).
- **pnpm only** for `@xyflow/react` — commit `web/package.json` + root `pnpm-lock.yaml`; never create `web/package-lock.json`.
- **Schema-graph specifics:** the overview returns the whole schema-graph (it's small), so `nodes`/`edges` start complete; neighbor-expansion mostly matters for future capped/stored graphs but is wired correctly (merges edges, fetches missing nodes).
- **Deselect via pane click:** `onPaneClick` passes `""`; `GraphView` maps `""`→`null`. Keep that convention consistent between canvas and orchestrator.
- **Out of scope (deferred polish, later phase):** tree panel, realm/cluster drill-down, breadcrumbs, the floating search dropdown, edge-collapsing for 5+ multi-edges, animated edges. v1 is: render, select, detail, expand, legend-filter, layout-switch.
- **Out of scope (G1d):** running the stack + seed data.
