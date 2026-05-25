# Graph Layer G1c.3 — Web Graph Selector + X-Database Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Let users pick which graph the `/graph` view renders — the synthetic `schema` graph or any registered stored graph — and make stored graphs actually resolve by sending the `X-Database` header on graph requests.

**Why X-Database is required:** the server resolves a stored graph via `catalog.get_graph(database, name)` keyed on the `X-Database` header (G1b.2). The web graph SDK currently sends only `Authorization` + `content-type`, so stored-graph requests would 404. This plan adds `X-Database` (from the session's `database`) to all `/v1/graph` calls, and adds a selector UI fed by `GET /v1/graph` (which lists `schema` + registered graphs for that database).

**Tech Stack:** React 18, TanStack Query, the existing graph SDK/hooks/store from G1c.1–c.2.

**Reference:** `web/src/sdk/graph.ts` (client), `web/src/features/graph/useGraph.ts` (hooks), `web/src/features/graph/GraphView.tsx` (orchestrator), `web/src/features/graph/graph-store.ts` (`graph`/`setGraph`), `web/src/sdk/session.ts` (`useSession()` → `{endpoint, token, database}`).

**Working dir:** worktree `…/.claude/worktrees/feature+graph-layer`, web under `web/`. Verify: `cd web && npx tsc --noEmit`, `npx vitest run src/sdk/graph.test.ts`, `npm run build`.

---

## Task 1: thread `X-Database` through the graph SDK

**Files:** `web/src/sdk/graph.ts`, `web/src/sdk/graph.test.ts`.

- [ ] **Step 1: update `headers` + thread `database`.** Change the `headers` helper to accept an optional database and set `x-database`:
```ts
function headers(token: string, database?: string): Record<string, string> {
  const h: Record<string, string> = { authorization: `Bearer ${token}`, "content-type": "application/json" };
  if (database) h["x-database"] = database;
  return h;
}
```
Add `database?: string` to the shared arg types so every call can pass it. Update `BaseArgs` and `GraphArgs`:
```ts
type BaseArgs = { endpoint: string; token: string; database?: string };
type GraphArgs = BaseArgs & { graph: string };
```
Then in EVERY fetch function (`listGraphs`, `getOverview`, `getStats`, `getGraphSchema`, `getNode`, `getSubgraph`, `searchNodes`, `expandNeighbors`), change `headers(args.token)` → `headers(args.token, args.database)`. (No other logic changes.)

- [ ] **Step 2: add a test** asserting the header is sent. Append to `graph.test.ts`:
```ts
test("sends x-database header when database is set", async () => {
  mockFetch.mockResolvedValue(ok({ stats: { total_nodes: 0, total_relationships: 0, label_counts: {}, relationship_type_counts: {} }, nodes: [], edges: [] }));
  await getOverview({ ...base, database: "obs", graph: "kg" });
  const [, init] = mockFetch.mock.calls[0];
  expect(init.headers["x-database"]).toBe("obs");
});

test("omits x-database header when database is absent", async () => {
  mockFetch.mockResolvedValue(ok([{ name: "schema", kind: "schema", description: "" }]));
  await listGraphs(base); // base has no database
  const [, init] = mockFetch.mock.calls[0];
  expect(init.headers["x-database"]).toBeUndefined();
});
```
(`base` is the existing `{ endpoint, token }` test constant — it has no `database`, so the omit test holds.)

- [ ] **Step 3:** `cd web && npx vitest run src/sdk/graph.test.ts` → all pass (prior + 2 new). `npx tsc --noEmit` clean.

- [ ] **Step 4: Commit:**
```bash
git add web/src/sdk/graph.ts web/src/sdk/graph.test.ts
git commit -m "feat(web/graph): send X-Database header on /v1/graph requests"
```

---

## Task 2: pass `database` from hooks + add the selector

**Files:** `web/src/features/graph/useGraph.ts`, `web/src/features/graph/GraphView.tsx`.

- [ ] **Step 1: hooks pass `database`.** In `useGraph.ts`, every hook currently destructures `const { endpoint, token } = useSession();`. Change to `const { endpoint, token, database } = useSession();` and add `database` to each SDK call's args object (`listGraphs`, `getOverview`, `getStats`, `getGraphSchema`, `getSubgraph`, and the imperative `expandNeighbors`/`searchNodes`). Also add `database` to each `useQuery` `queryKey` (so switching database refetches) — e.g. `queryKey: ["graph", "overview", endpoint, database, graph, realm ?? null, limit]`. For `useGraphList`: `queryKey: ["graph", "list", endpoint, database]`, `queryFn: () => listGraphs({ endpoint, token, database })`.

- [ ] **Step 2: selector UI in `GraphView.tsx`.** Add `useGraphList` to the imports from `./useGraph`. Inside `GraphView`, call `const graphList = useGraphList();` and render a `<select>` overlay at top-left. Place this just inside the `<div className="relative flex-1">`, before `<GraphCanvas .../>`:
```tsx
<div className="pointer-events-auto absolute left-3 top-3 z-20">
  <select
    value={graph}
    onChange={(e) => setGraph(e.target.value)}
    className="rounded-md border border-border bg-background/90 px-2 py-1 text-xs shadow-sm backdrop-blur focus:outline-none"
    title="Graph"
  >
    {(graphList.data && graphList.data.length > 0
      ? graphList.data
      : [{ name: "schema", kind: "schema", description: "" }]
    ).map((g) => (
      <option key={g.name} value={g.name}>
        {g.name}
        {g.kind === "stored" ? " (stored)" : ""}
      </option>
    ))}
  </select>
</div>
```
`graph` and `setGraph` already come from `useGraphStore` in this component. Switching the select sets the store graph → `useGraphOverview(graph)` refetches → the existing `useEffect` resets `nodes`/`edges`. No other wiring needed.

- [ ] **Step 3:** `cd web && npx tsc --noEmit` clean; `cd web && npm run build` succeeds.

- [ ] **Step 4: Commit:**
```bash
git add web/src/features/graph/useGraph.ts web/src/features/graph/GraphView.tsx
git commit -m "feat(web/graph): graph selector + database-scoped graph hooks"
```

---

## Task 3: verify
- [ ] `cd web && npx tsc --noEmit` clean.
- [ ] `cd web && npx vitest run src/sdk/graph.test.ts src/sdk/graph-layout.test.ts src/features/graph/graph-store.test.ts` → all pass.
- [ ] `cd web && npm run build` → succeeds.

---

## Self-review notes
- **X-Database is the load-bearing fix:** without it stored-graph requests 404. The `schema` graph ignores the header server-side (spans the catalog), so adding it is harmless there.
- **No new dep, no new UI primitive** — a styled native `<select>` keeps it minimal (the app has no Select primitive; a native select is robust and accessible).
- **Refetch on switch:** `database` + `graph` are in the query keys, so changing either refetches; `setGraph` already clears selection + label filter in the store.
- **Out of scope:** a fancy combobox, per-graph realm pickers, create-graph-from-UI. This is the minimal "see + switch graphs" connector.
