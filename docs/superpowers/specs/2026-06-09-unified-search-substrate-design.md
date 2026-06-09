# Unified `/v1/search` Substrate — Design

**Date:** 2026-06-09
**Status:** Approved (autonomous — driving the shared-substrate program end to end)
**Scope:** Make `/v1/search` the single retrieval substrate. One shared in-process `unified_search()` dispatches by `mode` (`data` | `memory` | `graph`); the HTTP endpoint, the MCP tools, and the web UI all route through it. Memory recall keeps its rich orchestrator — nothing is dumbed down.

## Program context

**Piece 1 of the 4-piece shared-substrate program** (build order 4→1→3→2). Piece 4 (artifacts-as-graph) shipped + merged. This piece is the headline: "route agent memory-recall + graph + MCP through `/v1/search` so agents share the substrate." Pieces 3 (Cypher) and 2 (dreaming-as-skill) follow.

## Problem

Today there are **three parallel retrieval paths** with no shared entry:
- `/v1/search` (`search.rs`) — lexical+vector RRF over a `scope`. Used by the UI keyword search.
- Memory recall (`memory_retrieve.rs::retrieve()`, `/v1/agent/memory/query`, MCP `memory_search`/`recall_memory`) — a **richer** orchestrator: RRF + graph-expansion + importance + recency + bi-temporal validity + point-in-time. Calls `retrieve()` in-process.
- Graph search (`graph_handler.rs::search`, `/v1/graph/:graph/search`) — text/label match over a graph's nodes. No MCP tool.

Agents (via MCP) and the UI (via HTTP) hit different code, so results, ranking, and provenance differ by caller. "Share the substrate" = one retrieval brain behind every caller.

## Decisions (locked)

| Decision | Choice |
|---|---|
| Shape of "sharing" | A **shared in-process `unified_search()` function** is the substrate. HTTP `/v1/search` and the MCP tools are thin wrappers over it — **not** MCP-over-HTTP (no round-trip; agents call the same function the endpoint does). |
| Modes | `data` (default; existing lexical+vector RRF), `memory` (delegates to `retrieve()`), `graph` (delegates to graph search). Explicit `mode` field; no auto-detection in v1. |
| Memory recall quality | **Untouched.** `memory` mode calls `retrieve()` as-is (all 6 blend factors, graph expansion, validity, `as_of`). No logic moves out of `memory_retrieve.rs`. |
| Backward compatibility | `/v1/search` with no `mode` == today's behavior, byte-for-byte for the `data` envelope. New envelope fields are **additive/optional**. `/v1/agent/memory/query` and `/v1/graph/:graph/search` stay. |
| MCP | **Refined during implementation:** memory tools (`memory_search`/`recall_memory`) keep calling `retrieve()` directly — that IS the shared substrate the `memory` arm delegates to, and the tools return a strictly richer payload than the lossy `UnifiedHit` envelope, so re-pointing them would regress recall. Instead add a `search` tool (`data` mode) and a `graph_search` tool (`graph` mode) so agents gain the data + graph modes through `unified_search`. |
| Context unification | A small `SearchCtx` (catalog, format, embedder, pool, tenant, principal) built by both the HTTP handler and the MCP tool ctx; `unified_search` builds each mode's needs from it (e.g. a `SharedToolCtx` for `retrieve()`). |
| UI | `data` stays the Explore default (current UX preserved). Client gains `mode`; the envelope change must not break `KymaExplore`. A lightweight memory/graph search affordance is in scope only if cheap; otherwise deferred. |

## Architecture

### The substrate: `unified_search()`

New module `crates/kyma-server/src/search/unified.rs` (split `search.rs` into a `search/` module: `mod.rs` keeps the data-mode legs/RRF, `unified.rs` is the dispatcher, `types.rs` the envelope).

```rust
pub struct SearchCtx { /* catalog, format, embedder handle, pool, tenant, principal */ }

pub enum SearchMode { Data, Memory, Graph }

pub struct UnifiedSearchRequest {
    pub query: String,
    pub mode: SearchMode,          // default Data
    pub limit: Option<usize>,
    // data:
    pub scope: Option<Scope>,
    pub time_range: Option<TimeRange>,
    // memory (passthrough to RetrieveRequest):
    pub realms: Vec<String>,
    pub memory_type: Option<String>,
    pub tags: Vec<String>,
    pub importance_min: Option<f32>,
    pub as_of: Option<String>,
    pub include_invalidated: bool,
    pub expand_hops: Option<u8>,
    // graph:
    pub graph: Option<String>,
    pub labels: Vec<String>,
}

pub struct UnifiedHit {
    pub score: f64,
    pub source: String,             // "db.table" (data), realm (memory), "db/graph" (graph)
    pub kind: String,               // "row" | "memory" | "node"
    pub id: Option<String>,
    pub title: Option<String>,
    pub row: Option<Value>,         // data
    pub content_preview: Option<String>, // memory
    pub memory_type: Option<String>,     // memory
}

pub struct UnifiedSearchResponse {
    pub mode: String,
    pub hits: Vec<UnifiedHit>,
    pub context: Option<String>,        // memory mode
    pub linked: Option<Vec<LinkedResource>>, // memory mode
    pub sources_searched: usize,
    pub elapsed_ms: u64,
}

pub async fn unified_search(ctx: &SearchCtx, req: UnifiedSearchRequest)
    -> Result<UnifiedSearchResponse, SearchError>;
```

- **`Data`**: the existing per-source lexical+vector RRF (the current `search.rs` body, with the WIP concurrency limiter + per-source memory budget kept). Each hit → `UnifiedHit { kind:"row", source, row, score }`.
- **`Memory`**: build a `SharedToolCtx` from `SearchCtx`, build a `RetrieveRequest` from the memory fields, call `retrieve()`. Map `RetrievedMemory` → `UnifiedHit { kind:"memory", … }`; pass through `context` + `linked`.
- **`Graph`**: resolve the graph (or search across all graphs in scope when `graph` is None) via the existing provider `search()`; map nodes → `UnifiedHit { kind:"node", source:"db/graph", … }`.

### HTTP `/v1/search`

`search_handler` builds `SearchCtx` from `QueryState` + principal/tenant, parses the (extended, backward-compatible) `SearchBody` into `UnifiedSearchRequest`, calls `unified_search`, serializes `UnifiedSearchResponse`. With no `mode`, the response keeps the current `{hits:[{source,score,row}], sources_searched, elapsed_ms}` keys (the new fields are simply absent/empty), so existing clients are unaffected.

### MCP tools (`kyma-mcp` + `agent/memory_tools.rs`)

- `memory_search` / `recall_memory` → call `unified_search(Memory)` (build `SearchCtx` from the tool's `SharedToolCtx`). Same inputs as today; identical results (it's the same `retrieve()` underneath, now via the shared entry).
- New `search` tool → `unified_search(Data)`: agents get hybrid lexical+vector data search (what the UI has).
- New `graph_search` tool → `unified_search(Graph)`: agents get graph node search.
- Register the two new tools in the dispatch table (`kyma-mcp/src/tools.rs`) and the bundled skill docs (the kyma skills) so agents discover them. Keep the agent-agnostic registry pattern.

### Client + UI

- `packages/client/src/search.ts`: `search()` request gains optional `mode` + the memory/graph fields; response type gains the optional envelope fields. Keep the existing default path working.
- `KymaExplore.tsx`: unchanged default (data). Optional: a small mode chip to target memory/graph (in scope only if it doesn't balloon the task; otherwise a follow-up). The `autoRun` WIP is preserved.

## Error handling
- Unknown `mode` → 400 with the allowed set.
- A mode's backend failing (e.g. no embedder) degrades the same way it does today (data: lexical-only; memory: keyword-only; graph: empty) — never 500 the whole request for a partial-source failure.
- `graph` mode with an unknown graph name → empty hits + a `note`, not an error.

## Testing
- **Rust unit**: `UnifiedSearchRequest` parsing/defaults (no mode → Data; backward-compatible body); envelope mapping for each mode (memory hit → UnifiedHit fields; data row → UnifiedHit).
- **Rust integration**: `unified_search(Memory)` returns the same memories as a direct `retrieve()` call (parity test — proves no regression); `unified_search(Data)` matches the pre-refactor `/v1/search` for a fixture; `unified_search(Graph)` finds a node.
- **MCP**: `memory_search` via the unified path returns the same shape as before (snapshot); the new `search`/`graph_search` tools dispatch and return hits.
- **HTTP**: `/v1/search` with no mode == legacy response keys; `mode:"memory"` returns `context`+memory hits.
- **Client/React**: `search()` with `mode` round-trips; KymaExplore still renders data results with the new envelope.

## Out of scope / deferred
- Auto mode-detection in `/v1/search` (explicit `mode` for v1).
- Cross-mode fusion (one query hitting data+memory+graph and merging) — modes are dispatched, not blended.
- Migrating `/v1/agent/memory/query` callers off it (it stays).
- Cypher (Piece 3), dreaming-as-skill (Piece 2).

## File touch list (anticipated)
- `crates/kyma-server/src/search.rs` → split into `search/{mod.rs,unified.rs,types.rs}` (mod.rs = data legs + RRF, kept).
- `crates/kyma-server/src/agent/memory_tools.rs` — memory tools call `unified_search`.
- `crates/kyma-mcp/src/tools.rs` — register `search` + `graph_search`; re-point memory tools.
- `crates/kyma-server/src/lib.rs` — handler wiring (route unchanged).
- `packages/client/src/search.ts` — `mode` + envelope types.
- `packages/react/src/explore/KymaExplore.tsx` — keep data default; optional mode chip.
- Bundled skill docs — mention the new tools (agent-agnostic).
