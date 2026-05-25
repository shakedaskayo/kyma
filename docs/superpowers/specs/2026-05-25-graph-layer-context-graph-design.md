# Kyma Graph Layer — First-Class Property-Graph + Context Graph UI — Master Design

**Status:** draft 2026-05-25. Decomposes into 4 phase plans (G1–G4). Each phase gets its own implementation plan under `docs/superpowers/plans/`.

**Relationship to other specs:** This is an engine + web-UI feature track, sibling to the v1 production-readiness program (`2026-05-19-kyma-v1-production-readiness-design.md`). It respects that program's locked decisions — most importantly **"the extent format freezes after format-v1"** (§4.2 below resolves graph indices as *sidecar index objects* to avoid touching the frozen format). The **self-learning / knowledge-ingestion** spec is sequenced *after* this one and writes into the property-graph construct defined here (§3); the seams are designed in, the population is out of scope (§1).

---

## 1. Goal & non-goals

### Goal

Give Kyma a **first-class property-graph** — registered in the catalog, queryable through the KQL engine, optimized in the execution layer — and a **Context Graph** web UI that renders it, modeled on the proven graph visualization built for the agentcy project (`@xyflow/react` canvas, force/grid/radial layout, cluster nodes, legend, tree panel, node-detail panel).

The deliverable is **end-to-end and runnable**: a developer opens the Kyma web app, navigates to **Graph**, and sees a live, interactive graph of their data — the catalog/schema graph (databases → tables → columns with real inferred edges) on day one, plus any registered stored property-graph.

Today graphs in Kyma are only a *query-time convention* (`graph-traverse` / `graph-shortest-path` compile to recursive CTEs; `explore_schema` returns a tabular schema dump; `find_references_to` infers shared-value edges). There is **no first-class graph in the catalog and no graph visualization**. This spec closes both gaps.

### Why this shape

- **One engine, tabular graphs.** Consistent with `docs/graphs.md`: edges are rows, traversal composes with the rest of KQL/SQL. We add a *registration + execution* layer on top, not a separate graph store.
- **The schema-graph gives real data on day one.** No ingestion needed to demo or test — the catalog already describes tables, columns, and shared-value relationships.
- **Self-learning plugs straight in.** The follow-on knowledge-ingestion spec registers a graph and appends nodes/edges; the same endpoints and UI render it unchanged.

### Non-goals (this spec)

- **Knowledge/memory ingestion** (connectors → entity/edge extraction, embedding/semantic-search population, entity resolution/dedup). That is the *next* spec. We expose the seams (registration, `props:dynamic`, `source_type`/`source_id` columns, a search endpoint) but do not populate them from connectors here.
- **A new extent format field for graph indices.** Indices are sidecar objects (§4.2).
- **Multi-node graph distribution / cross-region graph federation** (carried from the v1 program's non-goals).
- **Vector/semantic ranking inside `search`** at v1 (text/label/realm filtering only; the endpoint shape leaves room for vector scoring later).

---

## 2. Architecture overview

```
                         ┌─────────────────────────────────────────┐
  web/ (Tauri + React)   │  /graph route → features/graph/          │
                         │   GraphView · GraphCanvas (@xyflow)      │
                         │   TreePanel · NodeDetail · Legend · …    │
                         │   sdk/graph.ts (react-query) · zustand   │
                         └───────────────┬─────────────────────────┘
                                         │  JSON over HTTP (Bearer + X-Database)
                         ┌───────────────▼─────────────────────────┐
  kyma-server            │  /v1/graph/* handlers                    │
                         │   GraphProvider (trait)                  │
                         │    ├─ SchemaGraphProvider (catalog)      │
                         │    └─ StoredGraphProvider (registered)   │
                         └───────────────┬─────────────────────────┘
                                         │
              ┌──────────────────────────┼───────────────────────────┐
              ▼                          ▼                           ▼
   kyma-catalog                   kyma-kql                    kyma-exec
   graphs registration   make-graph / graph-match   frontier-materialized
   (graph_config)         extended graph-traverse    traversal + sidecar
                                                      graph indices
```

New crate **`kyma-graph`** owns the `GraphProvider` trait + the two providers + the wire types; `kyma-server` mounts the HTTP routes; `kyma-catalog` gains graph registration; `kyma-kql` / `kyma-exec` gain the KQL surface + execution.

---

## 3. The property-graph construct (engine)

### 3.1 Node & edge model

A **graph** is a named catalog object binding a **node table** and an **edge table** with declared column roles.

**Conventional node-table schema:**

```
id:string, labels:string[], props:dynamic, realm:string,
created_at:timestamp, updated_at:timestamp,
source_type:string, source_id:string
```

**Conventional edge-table schema:**

```
id:string, src:string, dst:string, type:string, props:dynamic, realm:string,
valid_from:timestamp, valid_to:timestamp   (the two timestamps optional — temporal graphs)
```

`props` uses Kyma's `dynamic` JSON column (the plan confirms the exact type token). `labels` is a string array (fallback: comma-delimited string if arrays are awkward in a given path — resolved in the G1 plan). `realm` is an optional partition dimension (mirrors agentcy); it defaults to the database name, and the schema-graph uses `realm = database`.

Column roles are *declared*, so a graph can be registered over **existing** tables whose columns are named differently — the conventional schema is the zero-config default, not a hard requirement.

### 3.2 Catalog registration

A new catalog object (Postgres-backed `graphs` table + `Catalog` trait methods):

```rust
struct GraphRegistration {
    name: String,
    database: String,
    node_table: String,
    edge_table: String,
    // column roles (default to the conventional schema)
    id_col: String, label_col: String,
    src_col: String, dst_col: String, type_col: String,
    realm_col: Option<String>,
    temporal: Option<TemporalCols>,   // { valid_from, valid_to }
}

// Catalog trait additions
async fn create_graph(&self, reg: GraphRegistration) -> Result<()>;
async fn lookup_graph(&self, database: &str, name: &str) -> Result<Option<GraphRegistration>>;
async fn list_graphs(&self, database: &str) -> Result<Vec<GraphRegistration>>;
async fn drop_graph(&self, database: &str, name: &str) -> Result<()>;
```

Stored as its own `graphs` table (clean separation; avoids overloading `tables.config`). Migration under `crates/kyma-catalog/migrations/`.

**CLI:** `kyma-cli create-graph --db <db> --name <g> --nodes <tbl> --edges <tbl> [--id-col … --src-col … …]`, plus `list-graphs` / `drop-graph`. Defaults apply when tables follow the conventional schema.

### 3.3 The schema-graph (synthetic, no registration)

The catalog itself is exposed as a read-only graph (`SchemaGraphProvider`, §5):

- **Nodes:** one per table (`labels: ["Table"]`, props = column list, row-count estimate, retention); optionally per-column nodes behind a depth toggle.
- **Edges:** `REFERENCES` edges inferred via the existing `find_references_to` column-stats scan (shared values across columns).
- **Realm:** the database name.

This is what renders on first launch — zero ingestion required — and is the primary test fixture.

---

## 4. KQL surface & execution (engine)

### 4.1 KQL surface (Phase G2)

- **`make-graph`** — `Edges | make-graph src --> dst with Nodes on id` produces an in-query graph handle (ADX-style), so downstream operators see a graph rather than an edge list.
- **`graph-match`** — `G | graph-match (a)-[e:TYPE]->(b) where <pred> project a.x, e.y, b.z` → ordinary tabular result. Supports fixed-length patterns at v1; variable-length defers to `graph-traverse`.
- **Extended `graph-traverse`** — edge-type filter, per-hop property predicates, and a multi-source seed set (today it takes a single literal source).

All parse through the existing parse-to-`QueryState` pipeline (`crates/kyma-kql/src/parser.rs`, `state.rs`). `graph-match` and the extended traverse lower to the frontier plan (§4.2) rather than a monolithic recursive CTE.

### 4.2 Execution (Phase G3)

- **Frontier-materialized traversal** — a custom `ExecutionPlan` node in `kyma-exec` that materializes each hop's frontier and applies pruning per hop, replacing the recursive-CTE expansion. KQL `graph-traverse` / `graph-shortest-path` / `graph-match` lower to it. Correctness is validated against the existing recursive-CTE implementation as a reference oracle.
- **Block-level graph indices** — per-extent equality indices on `src` / `dst` (edge tables) and `id` (node tables), so each hop skips extents with no matching keys. **Constraint:** these are maintained as **sidecar index objects** in object storage keyed to the extent, *not* a new extent-format field — this honors the v1 program's "extent format frozen after format-v1" decision regardless of format-v1 status. They build on the existing per-extent equality/token-index concepts.
- **Cycle handling** — a visited-set carried in the frontier node plus the `max-hops` guard guarantees termination; `graph-shortest-path` already dedups by min-depth.

---

## 5. Server: `GraphProvider` + `/v1/graph/*`

### 5.1 Trait & providers (crate `kyma-graph`)

```rust
#[async_trait]
trait GraphProvider {
    async fn overview(&self, realm: Option<&str>, limit: usize) -> Result<GraphPayload>;          // stats + capped nodes + edges
    async fn node(&self, id: &str) -> Result<Option<GraphNode>>;
    async fn neighbors(&self, ids: &[String], dir: Direction,
                       only_internal: bool, limit: usize) -> Result<EdgeExpansion>;                 // { edges, new_node_ids }
    async fn subgraph(&self, id: &str, depth: usize) -> Result<GraphPayload>;
    async fn search(&self, text: &str, labels: &[String],
                    realm: Option<&str>, limit: usize, offset: usize) -> Result<SearchHits>;
    async fn stats(&self, realm: Option<&str>) -> Result<GraphStats>;
    async fn schema(&self) -> Result<GraphSchema>;                                                  // node_kinds, edge_types, property_keys
}
```

- **`SchemaGraphProvider`** — computes from the catalog (§3.3). No registration.
- **`StoredGraphProvider`** — over a `GraphRegistration`. Phase G1 compiles ops to existing KQL/SQL (`node` = id lookup; `neighbors` = 1-hop edge scan; `subgraph` = `graph-traverse` depth N + edge collect; `search` = text/label filter; `stats` = `summarize count() by label / type`). Phases G2–G3 swap the hot paths to `graph-match` / the frontier plan.

### 5.2 Wire types (mirror agentcy exactly)

```ts
GraphNode         { id, labels: string[], properties: Record<string,unknown>,
                    metadata: { created_at, updated_at, source_type?, source_id?, realm } }
GraphRelationship { id, source_id, target_id, relationship_type, properties }
GraphStats        { total_nodes, total_relationships,
                    label_counts: Record<string,number>, relationship_type_counts: Record<string,number> }
```

JSON in/out (payloads are small and shaped — not Arrow). Same auth as `/v1/query`: `Authorization: Bearer …` + `X-Database`.

### 5.3 Endpoints

| Endpoint | Body / query | Returns |
|---|---|---|
| `GET /v1/graph` | — | registered graphs + synthetic schema-graph(s) |
| `GET /v1/graph/{g}/overview` | `?realm&limit` | `{ stats, nodes[], edges[] }` |
| `GET /v1/graph/{g}/nodes/{id}` | — | `GraphNode` |
| `POST /v1/graph/{g}/neighbors` | `{ node_ids[], direction, only_internal, limit }` | `{ edges[], new_node_ids[] }` |
| `GET /v1/graph/{g}/nodes/{id}/subgraph` | `?depth` | `{ nodes[], edges[] }` |
| `POST /v1/graph/{g}/search` | `{ text, labels[], realm, limit, offset }` | `{ hits[], total, took_ms }` |
| `GET /v1/graph/{g}/stats` | `?realm` | `GraphStats` |
| `GET /v1/graph/{g}/schema` | — | `{ node_kinds[], edge_types[], property_keys{} }` |

---

## 6. Web UI: the Context Graph view

### 6.1 Structure

- Route `web/src/routes/_app.graph.tsx` → feature dir `web/src/features/graph/`. Add `@xyflow/react`. Add a **Graph** link to `web/src/app/shell.tsx`.
- Components (ported/adapted from agentcy onto Kyma's shadcn-style `components/ui/`):
  - **`GraphView`** — orchestrator: graph selector (registered graphs + schema-graph), realm filter, loads `overview`.
  - **`GraphCanvas`** — `@xyflow/react` canvas. Port the pure-JS `graph-layout.ts` (force / grid / radial; deterministic placement, label cohesion, overlap prevention). Custom `GraphNode` + `ClusterNode`. Edge collapsing for multi-edges. Hover/select → highlight neighbors + dim others.
  - **`CanvasToolbar`** — layout picker, fit/zoom, breadcrumbs, relationship-type filter.
  - **`GraphLegend`** — label counts; click to filter.
  - **`GraphTreePanel`** (left) — realm → label → node tree, with search.
  - **`NodeDetailPanel`** (right) — properties, relationships grouped by type with direction markers, **expand-neighbors** action.
  - **`GraphSearchBar`** — debounced node search (300 ms, ≥2 chars).
- **`web/src/sdk/graph.ts`** — typed `/v1/graph/*` client + react-query hooks: `useGraphList`, `useGraphOverview`, `useNeighbors`, `useNodeDetail`, `useGraphSearch`, `useGraphStats`, `useGraphSchema`.
- **`web/src/features/graph/graph-store.ts`** (zustand) — selection, highlight set, label/type filters, expanded-node set, layout choice.

### 6.2 Styling

Port `getLabelColor` / `getRelationshipColor` (deterministic hash + presets) but retune presets to Kyma's domain (`Table`, `Column`, `Service`, `Database`, `Trace`…) and support Kyma web's existing light/dark themes (agentcy's was dark-only).

### 6.3 Agent-console graph renderers (Phase G4)

In the existing `web/src/features/agent` console, add tool-renderers for `graph_traverse`, `explore_schema`, and the new `graph_match` / `subgraph` tool results → a compact, read-only embedded `GraphCanvas` + an "open in Graph view" action. Mirrors agentcy's `tool-renderers/graph/*`.

---

## 7. Local run + seed (the "test it" deliverable)

- **One-command local stack:** `kyma-bin` server + Postgres catalog + object store (MinIO or local-fs `object_store`) + web `vite` dev server pointed at it. Reuse `docker-compose.yml` + `scripts/` where possible; add a `make graph-dev` (or script) target.
- **Seed script:** creates a sample database with FK-shaped tables (so the schema-graph shows real `REFERENCES` edges) and registers a small sample property-graph (e.g. a service-call graph) so the stored-graph provider has data.
- **Acceptance:** at the end of Phase G1, the stack is brought up for real and the Context Graph is verified rendering live data (nodes visible, click → detail, expand neighbors works).

---

## 8. Testing strategy

TDD throughout (repo rigor; gauntlet is the connective tissue).

- **Engine** — catalog-registration unit tests; provider golden SQL/KQL; `graph-match` parser tests; frontier-plan correctness vs the recursive-CTE reference oracle; index-pruning tests (verify extents skipped).
- **Server** — integration tests hitting every `/v1/graph/*` endpoint against a seeded catalog (both providers).
- **Web** — vitest for `sdk/graph.ts`, the layout function, and the store; a Playwright e2e smoke: open `/graph` → see nodes → click → detail → expand neighbors.
- **Gauntlet** — graph traversal/match correctness + perf scenarios added in Phase G3.

---

## 9. Phase decomposition

Full-engine target (the chosen "C" scope), delivered runnable-first. Each phase → its own plan under `docs/superpowers/plans/`.

- **G1 — runnable skeleton.** Catalog graph registration + CLI; `kyma-graph` crate with `GraphProvider`; `SchemaGraphProvider` + basic `StoredGraphProvider` (ops compiled to existing KQL/SQL); `/v1/graph/*` endpoints; web Context Graph view (xyflow + ported layout + panels + SDK + store + nav); local run + seed. **Ends with the stack up and the Context Graph rendering real schema-graph data — this is where we "start the web UI and stack to test it."**
- **G2 — KQL graph surface.** `make-graph` + `graph-match`; extended `graph-traverse`; endpoints switch hot paths to them.
- **G3 — execution optimization.** Frontier-materialized traversal `ExecutionPlan`; sidecar block-level graph indices; cycle detection; gauntlet perf + correctness.
- **G4 — agent integration + polish.** Agent-console graph tool-renderers; stats panels; saved graph views.

We write **and execute G1 first**; G2–G4 each get their own plan afterward.

---

## 10. Open items to resolve in the G1 plan

- Exact `dynamic` type token and how `labels:string[]` is represented across the ingest/query paths (array vs comma-string).
- Whether per-column schema-graph nodes are worth the clutter, or columns stay as node props with a depth toggle.
- The precise object-storage key layout for sidecar graph indices (G3, but decided early so G1 schemas don't fight it).
- Confirm current format-v1 status to double-check the sidecar-index decision is still the right call (vs folding into format-v1 if still open).
