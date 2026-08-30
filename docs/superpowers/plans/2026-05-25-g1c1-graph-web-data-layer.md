# Graph Layer G1c.1 — Web Data Layer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the pensieve web app's typed client + data plumbing for the Context Graph — an `sdk/graph.ts` over the `/v1/graph/*` endpoints, the ported pure-JS graph layout, a zustand graph store, and react-query hooks — all unit-tested, with no rendering yet (G1c.2 builds the view on top).

**Architecture:** Mirror the existing pensieve web conventions exactly: per-module fetch helpers (`headers`/`handleResponse`/`base`) like `sdk/dashboards.ts`, `useSession.getState()` for endpoint/token, `useQuery` for reads (route-level pattern), and a `create<T>()(...)` zustand store. The graph layout is ported verbatim from the agentcy project (it is dependency-free) with the label-color palette retuned to pensieve's domain.

**Tech Stack:** React 18, TypeScript, `@tanstack/react-query` ^5, `zustand` ^5, Vitest (jsdom). No new runtime dependency in this plan (`@xyflow/react` is added in G1c.2).

**Reference:** spec `docs/superpowers/specs/2026-05-25-graph-layer-context-graph-design.md` (§5.2 wire types, §5.3 endpoints, **§11 the real contract** — search returns `{hits,total,limit,offset}`, subgraph returns a full payload with `stats`, `Direction` is lowercase). The backend (G1a) is live on these endpoints.

---

## File structure

- Create `web/src/sdk/graph.ts` — types + typed fetch functions for `/v1/graph/*`.
- Create `web/src/sdk/graph.test.ts` — fetch + error tests (mock `fetch`).
- Create `web/src/sdk/graph-layout.ts` — ported pure-JS layout (`computeLayout`, `forceDirectedLayout`, `gridLayout`, `radialLayout`) + `getLabelColor`/`getRelationshipColor`.
- Create `web/src/sdk/graph-layout.test.ts` — deterministic layout + color tests.
- Create `web/src/features/graph/graph-store.ts` — zustand UI state (selection, layout, filters, expansion set).
- Create `web/src/features/graph/graph-store.test.ts` — store tests.
- Create `web/src/features/graph/useGraph.ts` — react-query hooks.
- Create `web/src/features/graph/useGraph.test.ts` — hook smoke (query key + fn wiring) OR fold into graph.test.ts (see Task 4).

All under the MAIN repo layout but edited in the worktree at `/Users/shakedaskayo/shaked/projects/pensieve/.claude/worktrees/feature+graph-layer`. Run web commands from `web/` inside the worktree: `cd web && npm run test`, `npm run typecheck`.

---

## Task 1: `sdk/graph.ts` — types + typed fetch client

**Files:** Create `web/src/sdk/graph.ts`, `web/src/sdk/graph.test.ts`.

- [ ] **Step 1: Write the failing test** — `web/src/sdk/graph.test.ts`:

```ts
import { beforeEach, expect, test, vi } from "vitest";
import { getOverview, getNode, searchNodes, expandNeighbors } from "./graph";

const mockFetch = vi.fn();
beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
});

function ok(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

const base = { endpoint: "http://localhost:7070", token: "tok" };

test("getOverview GETs the overview endpoint with auth + limit", async () => {
  mockFetch.mockResolvedValue(ok({ stats: { total_nodes: 0, total_relationships: 0, label_counts: {}, relationship_type_counts: {} }, nodes: [], edges: [] }));
  await getOverview({ ...base, graph: "schema", limit: 500 });
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/overview?limit=500");
  expect(init.headers.authorization).toBe("Bearer tok");
});

test("getNode encodes the node id in the path", async () => {
  mockFetch.mockResolvedValue(ok({ id: "default::orders", labels: ["Table"], properties: {}, metadata: { created_at: "", updated_at: "", realm: "default" } }));
  await getNode({ ...base, graph: "schema", id: "default::orders" });
  const [url] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/nodes/default%3A%3Aorders");
});

test("searchNodes POSTs a JSON body", async () => {
  mockFetch.mockResolvedValue(ok({ hits: [], total: 0, limit: 20, offset: 0 }));
  await searchNodes({ ...base, graph: "schema", text: "ord" });
  const [url, init] = mockFetch.mock.calls[0];
  expect(url).toBe("http://localhost:7070/v1/graph/schema/search");
  expect(init.method).toBe("POST");
  expect(JSON.parse(init.body)).toMatchObject({ text: "ord" });
});

test("expandNeighbors POSTs node_ids + direction", async () => {
  mockFetch.mockResolvedValue(ok({ edges: [], new_node_ids: [] }));
  await expandNeighbors({ ...base, graph: "schema", nodeIds: ["default::orders"], direction: "both" });
  const [, init] = mockFetch.mock.calls[0];
  expect(JSON.parse(init.body)).toMatchObject({ node_ids: ["default::orders"], direction: "both" });
});

test("throws on 401", async () => {
  mockFetch.mockResolvedValue(new Response("no", { status: 401 }));
  await expect(getOverview({ ...base, graph: "schema" })).rejects.toThrow(/unauthorized/i);
});
```

- [ ] **Step 2: Run to verify it fails** — `cd web && npx vitest run src/sdk/graph.test.ts` → FAIL (module not found).

- [ ] **Step 3: Implement `web/src/sdk/graph.ts`:**

```ts
// Typed client for the pensieve graph layer (`/v1/graph/*`). JSON in/out.
// Mirrors the per-module fetch-helper convention used by `sdk/dashboards.ts`.

export type Props = Record<string, unknown>;

export interface NodeMetadata {
  created_at: string;
  updated_at: string;
  source_type?: string | null;
  source_id?: string | null;
  realm: string;
}
export interface GraphNode {
  id: string;
  labels: string[];
  properties: Props;
  metadata: NodeMetadata;
}
export interface GraphRelationship {
  id: string;
  source_id: string;
  target_id: string;
  relationship_type: string;
  properties: Props;
}
export interface GraphStats {
  total_nodes: number;
  total_relationships: number;
  label_counts: Record<string, number>;
  relationship_type_counts: Record<string, number>;
}
export interface GraphPayload {
  stats: GraphStats;
  nodes: GraphNode[];
  edges: GraphRelationship[];
}
export interface EdgeExpansion {
  edges: GraphRelationship[];
  new_node_ids: string[];
}
export interface SearchHits {
  hits: GraphNode[];
  total: number;
  limit: number;
  offset: number;
}
export interface GraphSchema {
  node_kinds: string[];
  edge_types: string[];
  property_keys: Record<string, string[]>;
}
export interface GraphRef {
  name: string;
  kind: string;
  description: string;
}
export type Direction = "forward" | "backward" | "both";

type BaseArgs = { endpoint: string; token: string };
type GraphArgs = BaseArgs & { graph: string };

function headers(token: string): Record<string, string> {
  return { authorization: `Bearer ${token}`, "content-type": "application/json" };
}
function base(endpoint: string): string {
  return endpoint.replace(/\/$/, "");
}
async function handleResponse<T>(res: Response): Promise<T> {
  if (res.status === 401 || res.status === 403) throw new Error(`unauthorized (${res.status})`);
  if (res.status === 404) throw new Error("not found");
  if (!res.ok) {
    const snippet = await res.text().then((t) => t.slice(0, 200)).catch(() => "");
    throw new Error(`request failed: ${res.status}${snippet ? ` — ${snippet}` : ""}`);
  }
  return res.json() as Promise<T>;
}

export async function listGraphs(args: BaseArgs): Promise<GraphRef[]> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph`, { headers: headers(args.token) });
  return handleResponse<GraphRef[]>(res);
}

export async function getOverview(
  args: GraphArgs & { realm?: string; limit?: number },
): Promise<GraphPayload> {
  const q = new URLSearchParams();
  if (args.realm) q.set("realm", args.realm);
  if (args.limit != null) q.set("limit", String(args.limit));
  const qs = q.toString();
  const res = await fetch(
    `${base(args.endpoint)}/v1/graph/${args.graph}/overview${qs ? `?${qs}` : ""}`,
    { headers: headers(args.token) },
  );
  return handleResponse<GraphPayload>(res);
}

export async function getStats(args: GraphArgs & { realm?: string }): Promise<GraphStats> {
  const qs = args.realm ? `?realm=${encodeURIComponent(args.realm)}` : "";
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/stats${qs}`, {
    headers: headers(args.token),
  });
  return handleResponse<GraphStats>(res);
}

export async function getGraphSchema(args: GraphArgs): Promise<GraphSchema> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/schema`, {
    headers: headers(args.token),
  });
  return handleResponse<GraphSchema>(res);
}

export async function getNode(args: GraphArgs & { id: string }): Promise<GraphNode> {
  const res = await fetch(
    `${base(args.endpoint)}/v1/graph/${args.graph}/nodes/${encodeURIComponent(args.id)}`,
    { headers: headers(args.token) },
  );
  return handleResponse<GraphNode>(res);
}

export async function getSubgraph(
  args: GraphArgs & { id: string; depth?: number },
): Promise<GraphPayload> {
  const qs = args.depth != null ? `?depth=${args.depth}` : "";
  const res = await fetch(
    `${base(args.endpoint)}/v1/graph/${args.graph}/nodes/${encodeURIComponent(args.id)}/subgraph${qs}`,
    { headers: headers(args.token) },
  );
  return handleResponse<GraphPayload>(res);
}

export async function searchNodes(
  args: GraphArgs & { text: string; labels?: string[]; realm?: string; limit?: number; offset?: number },
): Promise<SearchHits> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/search`, {
    method: "POST",
    headers: headers(args.token),
    body: JSON.stringify({
      text: args.text,
      labels: args.labels ?? [],
      realm: args.realm,
      limit: args.limit ?? 20,
      offset: args.offset ?? 0,
    }),
  });
  return handleResponse<SearchHits>(res);
}

export async function expandNeighbors(
  args: GraphArgs & { nodeIds: string[]; direction?: Direction; onlyInternal?: boolean; limit?: number },
): Promise<EdgeExpansion> {
  const res = await fetch(`${base(args.endpoint)}/v1/graph/${args.graph}/neighbors`, {
    method: "POST",
    headers: headers(args.token),
    body: JSON.stringify({
      node_ids: args.nodeIds,
      direction: args.direction ?? "both",
      only_internal: args.onlyInternal ?? false,
      limit: args.limit ?? 200,
    }),
  });
  return handleResponse<EdgeExpansion>(res);
}
```

- [ ] **Step 4: Run to verify pass** — `cd web && npx vitest run src/sdk/graph.test.ts` → PASS (5 tests). Also `npx tsc --noEmit` clean for this file.

- [ ] **Step 5: Commit:**
```bash
git add web/src/sdk/graph.ts web/src/sdk/graph.test.ts
git commit -m "feat(web/graph): typed /v1/graph client + types"
```

---

## Task 2: `sdk/graph-layout.ts` — port the dependency-free layout

**Files:** Create `web/src/sdk/graph-layout.ts`, `web/src/sdk/graph-layout.test.ts`.

This file is **ported** from agentcy. **Read the source** at
`/Users/shakedaskayo/shaked/projects/agentcylabs/agentcy/frontend/lib/utils/graph-layout.ts`
and copy it into `web/src/sdk/graph-layout.ts` with these adaptations:

1. Replace the domain-type import (`import type { GraphNode, GraphRelationship } from "@/lib/types/graph"`) with `import type { GraphNode, GraphRelationship } from "./graph";` (the types created in Task 1 are structurally identical — `id`, `labels[]`, `properties`, `metadata`; relationships have `source_id`/`target_id`/`relationship_type`).
2. Keep `forceDirectedLayout`, `gridLayout`, `radialLayout`, `computeLayout`, and the `LayoutAlgorithm` type **verbatim** (they are pure, no external imports).
3. Keep `getLabelColor`/`getRelationshipColor` and the `generateColor` hash. **Retune the `LABEL_COLORS` map to pensieve's domain** — replace the agentcy connector labels with pensieve's: at minimum `Table: "#7ed957"` (pensieve green), `Column: "#60a5fa"`, `Database: "#a78bfa"`, `Service: "#f59e0b"`, `Trace: "#22d3ee"`. Unknown labels fall through to `generateColor` (keep that). Keep `REL_COLORS` but ensure `REFERENCES: "#94a3b8"` is present (the schema-graph's only edge type today).
4. Do NOT import anything from `@xyflow/react` here — the layout returns a plain `Map<string, {x:number;y:number}>`.

- [ ] **Step 1: Write the failing test** — `web/src/sdk/graph-layout.test.ts`:

```ts
import { expect, test } from "vitest";
import { computeLayout, getLabelColor, getRelationshipColor } from "./graph-layout";

test("grid layout positions every node with finite coords", () => {
  const ids = ["a", "b", "c", "d"];
  const pos = computeLayout("grid", ids.map((id) => ({ id })), [], 800, 600);
  expect(pos.size).toBe(4);
  for (const id of ids) {
    const p = pos.get(id)!;
    expect(Number.isFinite(p.x)).toBe(true);
    expect(Number.isFinite(p.y)).toBe(true);
  }
});

test("force layout is deterministic for the same input", () => {
  const nodes = [{ id: "a" }, { id: "b" }, { id: "c" }];
  const edges = [{ source_id: "a", target_id: "b" }, { source_id: "b", target_id: "c" }];
  const p1 = computeLayout("force", nodes, edges, 800, 600);
  const p2 = computeLayout("force", nodes, edges, 800, 600);
  expect(p1.get("a")).toEqual(p2.get("a"));
  expect(p1.get("c")).toEqual(p2.get("c"));
});

test("pensieve label colors resolve; unknown labels still get a color", () => {
  expect(getLabelColor("Table")).toBe("#7ed957");
  expect(typeof getLabelColor("SomethingUnknown")).toBe("string");
  expect(getRelationshipColor("REFERENCES")).toBe("#94a3b8");
});
```

- [ ] **Step 2: Run to verify it fails** — `cd web && npx vitest run src/sdk/graph-layout.test.ts` → FAIL (module not found).

- [ ] **Step 3: Create `web/src/sdk/graph-layout.ts`** by porting per the directives above. The `computeLayout` signature is:
```ts
export type LayoutAlgorithm = "force" | "grid" | "radial";
export function computeLayout(
  algorithm: LayoutAlgorithm,
  nodes: { id: string }[],
  relationships: { source_id: string; target_id: string }[],
  width: number,
  height: number,
): Map<string, { x: number; y: number }>;
```
If the agentcy `forceDirectedLayout` uses `Math.random()` anywhere (it should use the seeded jitter, not `Math.random`), REPLACE any `Math.random()` with the deterministic seed already present, so the determinism test passes. Report if you had to change anything for determinism.

- [ ] **Step 4: Run to verify pass** — `cd web && npx vitest run src/sdk/graph-layout.test.ts` → PASS (3 tests). `npx tsc --noEmit` clean.

- [ ] **Step 5: Commit:**
```bash
git add web/src/sdk/graph-layout.ts web/src/sdk/graph-layout.test.ts
git commit -m "feat(web/graph): port force/grid/radial layout + pensieve label palette"
```

---

## Task 3: `features/graph/graph-store.ts` — zustand UI state

**Files:** Create `web/src/features/graph/graph-store.ts`, `web/src/features/graph/graph-store.test.ts`.

- [ ] **Step 1: Write the failing test** — `web/src/features/graph/graph-store.test.ts`:

```ts
import { beforeEach, expect, test } from "vitest";
import { useGraphStore } from "./graph-store";

beforeEach(() => useGraphStore.getState().reset());

test("default state", () => {
  const s = useGraphStore.getState();
  expect(s.graph).toBe("schema");
  expect(s.layout).toBe("force");
  expect(s.selectedNodeId).toBeNull();
  expect(s.labelFilter).toBeNull();
});

test("select + clear node", () => {
  useGraphStore.getState().selectNode("default::orders");
  expect(useGraphStore.getState().selectedNodeId).toBe("default::orders");
  useGraphStore.getState().selectNode(null);
  expect(useGraphStore.getState().selectedNodeId).toBeNull();
});

test("toggle label filter", () => {
  useGraphStore.getState().setLabelFilter("Table");
  expect(useGraphStore.getState().labelFilter).toBe("Table");
  useGraphStore.getState().setLabelFilter(null);
  expect(useGraphStore.getState().labelFilter).toBeNull();
});

test("setLayout + setGraph", () => {
  useGraphStore.getState().setLayout("radial");
  expect(useGraphStore.getState().layout).toBe("radial");
  useGraphStore.getState().setGraph("schema");
  expect(useGraphStore.getState().graph).toBe("schema");
});
```

- [ ] **Step 2: Run to verify it fails** — `cd web && npx vitest run src/features/graph/graph-store.test.ts` → FAIL.

- [ ] **Step 3: Implement `web/src/features/graph/graph-store.ts`:**

```ts
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
```

- [ ] **Step 4: Run to verify pass** — `cd web && npx vitest run src/features/graph/graph-store.test.ts` → PASS (4 tests).

- [ ] **Step 5: Commit:**
```bash
git add web/src/features/graph/graph-store.ts web/src/features/graph/graph-store.test.ts
git commit -m "feat(web/graph): zustand graph UI store"
```

---

## Task 4: `features/graph/useGraph.ts` — react-query hooks

**Files:** Create `web/src/features/graph/useGraph.ts`, `web/src/features/graph/useGraph.test.tsx`.

Hooks wrap the `sdk/graph.ts` functions with `useQuery`, reading the live session reactively (the route-level pattern in `_app.explore.tsx`). Mutations (`expandNeighbors`) are exposed as a plain async helper bound to the current session, since expansion is an imperative user action whose results are merged into local view state by G1c.2 (not cached by react-query).

- [ ] **Step 1: Write the failing test** — `web/src/features/graph/useGraph.test.tsx`:

```tsx
import { beforeEach, expect, test, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import { useGraphOverview } from "./useGraph";
import { useSession } from "@/sdk/session";

const mockFetch = vi.fn();
beforeEach(() => {
  vi.stubGlobal("fetch", mockFetch);
  mockFetch.mockReset();
  useSession.setState({ endpoint: "http://localhost:7070", token: "tok", database: "default" } as never);
});

function wrapper({ children }: { children: React.ReactNode }) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
}

test("useGraphOverview fetches and returns the payload", async () => {
  const payload = { stats: { total_nodes: 1, total_relationships: 0, label_counts: { Table: 1 }, relationship_type_counts: {} }, nodes: [{ id: "default::t", labels: ["Table"], properties: {}, metadata: { created_at: "", updated_at: "", realm: "default" } }], edges: [] };
  mockFetch.mockResolvedValue(new Response(JSON.stringify(payload), { status: 200, headers: { "content-type": "application/json" } }));
  const { result } = renderHook(() => useGraphOverview("schema"), { wrapper });
  await waitFor(() => expect(result.current.data).toBeDefined());
  expect(result.current.data!.nodes).toHaveLength(1);
});
```

NOTE: this test needs `@testing-library/react`. Check `web/package.json` devDependencies; if `@testing-library/react` is absent, add it (`npm i -D @testing-library/react`). If the team prefers no RTL, FALL BACK to testing the hooks' underlying query options object instead — but RTL `renderHook` is the cleaner approach. Report which path you took.

- [ ] **Step 2: Run to verify it fails** — `cd web && npx vitest run src/features/graph/useGraph.test.tsx` → FAIL.

- [ ] **Step 3: Implement `web/src/features/graph/useGraph.ts`:**

```ts
import { useQuery } from "@tanstack/react-query";
import { useCallback } from "react";
import { useSession } from "@/sdk/session";
import {
  expandNeighbors as apiExpandNeighbors,
  getGraphSchema,
  getOverview,
  getStats,
  getSubgraph,
  listGraphs,
  searchNodes,
  type Direction,
  type EdgeExpansion,
  type GraphPayload,
} from "@/sdk/graph";

export function useGraphList() {
  const { endpoint, token } = useSession();
  return useQuery({
    queryKey: ["graph", "list", endpoint],
    queryFn: () => listGraphs({ endpoint, token }),
    enabled: Boolean(endpoint && token),
    staleTime: 5 * 60_000,
  });
}

export function useGraphOverview(graph: string, realm?: string, limit = 800) {
  const { endpoint, token } = useSession();
  return useQuery<GraphPayload>({
    queryKey: ["graph", "overview", endpoint, graph, realm ?? null, limit],
    queryFn: () => getOverview({ endpoint, token, graph, realm, limit }),
    enabled: Boolean(endpoint && token && graph),
  });
}

export function useGraphStats(graph: string, realm?: string) {
  const { endpoint, token } = useSession();
  return useQuery({
    queryKey: ["graph", "stats", endpoint, graph, realm ?? null],
    queryFn: () => getStats({ endpoint, token, graph, realm }),
    enabled: Boolean(endpoint && token && graph),
  });
}

export function useGraphSchema(graph: string) {
  const { endpoint, token } = useSession();
  return useQuery({
    queryKey: ["graph", "schema", endpoint, graph],
    queryFn: () => getGraphSchema({ endpoint, token, graph }),
    enabled: Boolean(endpoint && token && graph),
    staleTime: 5 * 60_000,
  });
}

export function useGraphSubgraph(graph: string, id: string | null, depth = 2) {
  const { endpoint, token } = useSession();
  return useQuery<GraphPayload>({
    queryKey: ["graph", "subgraph", endpoint, graph, id, depth],
    queryFn: () => getSubgraph({ endpoint, token, graph, id: id!, depth }),
    enabled: Boolean(endpoint && token && graph && id),
  });
}

/** Imperative neighbor expansion bound to the current session. */
export function useExpandNeighbors(graph: string) {
  const { endpoint, token } = useSession();
  return useCallback(
    (nodeIds: string[], direction: Direction = "both"): Promise<EdgeExpansion> =>
      apiExpandNeighbors({ endpoint, token, graph, nodeIds, direction }),
    [endpoint, token, graph],
  );
}

/** Imperative node search bound to the current session. */
export function useSearchNodes(graph: string) {
  const { endpoint, token } = useSession();
  return useCallback(
    (text: string) => searchNodes({ endpoint, token, graph, text }),
    [endpoint, token, graph],
  );
}
```

- [ ] **Step 4: Run to verify pass** — `cd web && npx vitest run src/features/graph/useGraph.test.tsx` → PASS. `npx tsc --noEmit` clean.

- [ ] **Step 5: Commit:**
```bash
git add web/src/features/graph/useGraph.ts web/src/features/graph/useGraph.test.tsx web/package.json web/package-lock.json
git commit -m "feat(web/graph): react-query hooks + imperative expand/search"
```

---

## Task 5: Verify the data layer end-to-end

- [ ] **Step 1:** `cd web && npm run typecheck` (or `npx tsc --noEmit`) → clean.
- [ ] **Step 2:** `cd web && npx vitest run src/sdk/graph.test.ts src/sdk/graph-layout.test.ts src/features/graph/graph-store.test.ts src/features/graph/useGraph.test.tsx` → all PASS.
- [ ] **Step 3:** `cd web && npm run lint` (if configured) → no new errors in the new files.
- [ ] **Step 4:** Confirm exports line up for G1c.2: `sdk/graph.ts` exports all types + fetch fns; `sdk/graph-layout.ts` exports `computeLayout`, `LayoutAlgorithm`, `getLabelColor`, `getRelationshipColor`; `graph-store.ts` exports `useGraphStore`; `useGraph.ts` exports the hooks.

---

## Self-review notes (for the executor)

- **Contract fidelity (spec §11):** `SearchHits` is `{hits,total,limit,offset}` (no `took_ms`); `getSubgraph` returns a full `GraphPayload` (with `stats`); `Direction` is the lowercase union. The fetch layer sends lowercase directions, so the server's strict-lowercase deserialization is satisfied.
- **No new runtime dep** in this plan. `@xyflow/react` is added in G1c.2. RTL (`@testing-library/react`) is a dev-dependency only (Task 4) — if the team rejects it, the fallback is noted.
- **Node-id encoding:** `getNode`/`getSubgraph` `encodeURIComponent` the `db::table` id so the `::` survives the path. (Matches the server's `:id` capture, which decoded fine in the G1a 404 test.)
- **Out of scope (G1c.2):** all rendering — `@xyflow/react`, `GraphCanvas`, custom node, `NodeDetailPanel`, legend/toolbar, `GraphView` orchestrator, the `/graph` route, and the shell nav link. Out of scope (G1d): local run + seed.
- **Layout port:** keep the three layout fns verbatim; only the `LABEL_COLORS` palette and the type import change. Determinism matters (a test asserts it) — strip any `Math.random()`.
