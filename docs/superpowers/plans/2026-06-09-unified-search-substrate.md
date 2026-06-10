# Unified `/v1/search` Substrate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. Steps use `- [ ]` checkboxes.

**Goal:** One shared in-process `unified_search()` dispatches by `mode` (data/memory/graph); HTTP `/v1/search`, the MCP tools, and the UI all route through it; memory recall keeps its rich `retrieve()` orchestrator.

**Architecture:** See `docs/superpowers/specs/2026-06-09-unified-search-substrate-design.md`. Additive + backward-compatible: no `mode` == today's data search byte-for-byte.

**Tech Stack:** Rust (kyma-server, kyma-mcp, DataFusion), TS (@kyma-ai/client, @kyma-ai/react).

**Preserve:** the uncommitted WIP in `search.rs` (concurrency limiter + per-source mem budget) and `KymaExplore.tsx` (`autoRun`) — do NOT revert; build on top.

---

### Task 1: Split `search.rs` into a `search/` module + envelope types (refactor, no behavior change)

**Files:** Move `crates/kyma-server/src/search.rs` → `crates/kyma-server/src/search/mod.rs`; add `crates/kyma-server/src/search/types.rs`.

- [ ] **Step 1** — `git mv crates/kyma-server/src/search.rs crates/kyma-server/src/search/mod.rs`. Confirm `mod search;` in `lib.rs` still resolves (Rust treats `search/mod.rs` as the module). Build: `cargo build -p kyma-server` → clean. This preserves the WIP diff (git mv keeps it).
- [ ] **Step 2** — Add `crates/kyma-server/src/search/types.rs` with the envelope types from the spec (`SearchMode`, `UnifiedHit`, `UnifiedSearchResponse`, and a `UnifiedSearchRequest`). Derive `Serialize`/`Deserialize`. `SearchMode` defaults to `Data` and deserializes from `"data"|"memory"|"graph"` (serde rename_all lowercase; `#[serde(default)]`). Add `pub mod types;` to `search/mod.rs`. Build clean.
- [ ] **Step 3** — Unit test in `types.rs`: a `SearchBody`-style JSON with no `mode` deserializes to `SearchMode::Data`; `"memory"`/`"graph"` parse; unknown → error. Run `cargo test -p kyma-server search::types`. Commit: `refactor(search): split into search/ module + unified envelope types`.

---

### Task 2: `unified_search()` with Data mode + re-point the HTTP handler (backward-compatible)

**Files:** `crates/kyma-server/src/search/unified.rs` (new), `crates/kyma-server/src/search/mod.rs`.

- [ ] **Step 1** — Write a failing parity test (`search/unified.rs` `#[cfg(test)]` or an integration test): given a fixture DB+table, `unified_search(ctx, {mode:Data, query, scope})` returns hits whose `(source,row)` match the current `search_handler` data path. (Build the fixture the way `search/mod.rs` tests or `kyma-catalog-sqlite` harness do; if no in-crate harness, assert the mapping from the existing per-source results to `UnifiedHit` instead.)
- [ ] **Step 2** — Implement `SearchCtx` (holds `catalog`, `format`, `pool: Option<PgPool>`, `tenant`, `principal`) and `pub async fn unified_search(ctx:&SearchCtx, req:UnifiedSearchRequest) -> Result<UnifiedSearchResponse,SearchError>`. For `Data`: call the existing per-source fan-out (extract the current handler's core into a `search_data(ctx, scope, query, time_range, limit)` fn in `mod.rs` returning `Vec<(source,score,row)>`), map each to `UnifiedHit{kind:"row",source,row:Some(row),score}`. Set `sources_searched`, `elapsed_ms`.
- [ ] **Step 3** — Re-point `search_handler` (`mod.rs`): build `SearchCtx` from `QueryState`+principal+tenant, parse the extended `SearchBody` (add optional `mode` + memory/graph fields, all `#[serde(default)]`) into `UnifiedSearchRequest`, call `unified_search`, serialize. CRITICAL backward-compat: when `mode==Data`, the JSON must still carry `hits:[{source,score,row}]`, `sources_searched`, `elapsed_ms` (use `#[serde(skip_serializing_if="Option::is_none")]` on the new fields so the legacy shape is unchanged).
- [ ] **Step 4** — Run the existing search tests + the new parity test: `cargo test -p kyma-server search`. Confirm the legacy `/v1/search` response is unchanged (add a serialization snapshot test asserting no `mode`/`context` keys appear for a data response). Commit: `feat(search): unified_search() dispatcher with data mode (backward-compatible)`.

---

### Task 3: Memory mode — delegate to `retrieve()` (parity, no regression)

**Files:** `crates/kyma-server/src/search/unified.rs`.

- [ ] **Step 1** — Failing test: `unified_search(ctx,{mode:Memory,query,realms,...})` returns hits that match a direct `retrieve(&shared, &RetrieveRequest{...})` call (same ids, same order) for a seeded memory store, and propagates `context` + `linked`. (Seed via the `MemoryWriter` test harness from `kyma-memory/tests/file_candidates_it.rs`.)
- [ ] **Step 2** — Implement the `Memory` arm: build a `SharedToolCtx` from `SearchCtx` (catalog, format, pool, `memory: None`), map the memory fields of `UnifiedSearchRequest` → `RetrieveRequest`, call `retrieve()`. Map `RetrievedMemory` → `UnifiedHit{kind:"memory", id:Some(id), title, content_preview, memory_type, source:realm, score}`; set `response.context`/`linked`. Do NOT touch `memory_retrieve.rs`.
- [ ] **Step 3** — Run `cargo test -p kyma-server search`. Parity test green. Commit: `feat(search): memory mode delegates to retrieve() (rich recall preserved)`.

---

### Task 4: Graph mode — delegate to graph provider search

**Files:** `crates/kyma-server/src/search/unified.rs`; reuse `graph_handler.rs` resolution.

- [ ] **Step 1** — Failing test: `unified_search(ctx,{mode:Graph,query,graph:Some("memory"),labels})` returns node hits for a seeded stored graph; with `graph:None`, searches across the graphs in scope and merges (cap per graph). Map nodes → `UnifiedHit{kind:"node", id, title:name, source:"db/graph", score}`.
- [ ] **Step 2** — Implement the `Graph` arm: reuse `graph_handler::resolve()` + `QueryEngineExecutor` (factor the resolver into a shared fn if it's private — expose `pub(crate)`), call `provider.search(query,labels,realm,limit,offset)`. For `graph:None`, enumerate graphs via the catalog (like the all-DB discovery) and merge. Errors/unknown-graph → empty hits + a note, never 500.
- [ ] **Step 3** — `cargo test -p kyma-server search`. Commit: `feat(search): graph mode delegates to graph provider search`.

---

### Task 5: MCP — route memory tools through the substrate + add `search`/`graph_search` tools

**Files:** `crates/kyma-server/src/agent/memory_tools.rs`, `crates/kyma-mcp/src/tools.rs`, bundled skill docs.

- [ ] **Step 1** — Failing test: a `search` MCP tool and a `graph_search` MCP tool exist in the dispatch table and return hits for seeded data/graph; `memory_search` still returns the same shape as before (snapshot) but now via `unified_search(Memory)`.
- [ ] **Step 2** — Implement: in `memory_tools.rs`, change the `memory_search`/`recall_memory` tool bodies to build a `SearchCtx` from `SharedToolCtx` and call `unified_search(Memory)` (mapping the unified hits back to the tool's current output shape so agents see no change). Add `tool_search` (Data) and `tool_graph_search` (Graph). Register both in `kyma-mcp/src/tools.rs` dispatch map. Keep the per-agent-kind registry pattern (agent-agnostic).
- [ ] **Step 3** — Update the bundled kyma skill docs (`integrations/.../skills/kyma-memory/SKILL.md` and any tool-listing) to mention `search` + `graph_search` — developer voice, no marketing. Run `cargo test -p kyma-server -p kyma-mcp agent`/`tools`. Commit: `feat(mcp): route memory search through unified_search; add search + graph_search tools`.

---

### Task 6: Client + UI — `mode` + envelope, data default preserved

**Files:** `packages/client/src/search.ts`, `packages/react/src/explore/KymaExplore.tsx`.

- [ ] **Step 1** — Failing client test (`packages/client` vitest if present, else a type-level test in react): `search(t, {query, mode:"memory", realms:[...]})` sends `mode` + memory fields; the response type includes optional `context`/`hits[].content_preview`. The default `search(t,{query,scope})` still works (no `mode`).
- [ ] **Step 2** — Extend `search.ts`: `HybridSearchRequest` gains optional `mode?: "data"|"memory"|"graph"` + the memory/graph fields; `HybridSearchResponse` gains optional `mode`, `context`, `linked`, and `hits[]` gains optional `kind`/`content_preview`/`memory_type`/`id`/`title`. Keep existing fields.
- [ ] **Step 3** — `KymaExplore.tsx`: keep `data` the default for the Search mode (current UX). Ensure the result renderer tolerates the new optional fields (no crash if `row` is absent for memory/node hits — guard the table render). Preserve the `autoRun` WIP. (A memory/graph mode chip is OPTIONAL — add only if < ~30 lines; else leave a `// TODO(piece-1 follow-up): memory/graph search chip` and ship data-compatible.)
- [ ] **Step 4** — `pnpm --filter @kyma-ai/client test` (if present) + `pnpm --filter @kyma-ai/react test`. Commit: `feat(client+react): search mode + unified envelope (data default preserved)`.

---

## Self-Review
- Substrate sharing → Tasks 2–5 (one `unified_search`, HTTP + MCP both call it). ✓
- Memory recall not regressed → Task 3 parity test; `memory_retrieve.rs` untouched. ✓
- Backward compatibility → Task 2 Step 3/4 (legacy envelope unchanged), Task 6 default path. ✓
- Agent-agnostic MCP registry → Task 5. ✓
- WIP preserved → Tasks 1 (git mv keeps search.rs WIP) + 6 (autoRun). ✓
- No placeholders except the explicitly-optional UI chip (Task 6 Step 3), gated on size.
