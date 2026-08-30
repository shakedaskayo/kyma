# Pensieve Agentic Memory — Master Design

**Status:** draft 2026-05-31. Decomposes into 7 phase plans (M0–M7). Each phase ships a working, testable slice on its own. M0 + M1 are the foundations and are built first.

**Relationship to other specs:** Extends the agent-engine track (`2026-05-28-agent-engine-and-skills-design.md`) — this is the "A5 sessions" item plus a new memory capability layered on the existing `/v1/agent/*` surface. Builds directly on the graph layer (`2026-05-25-graph-layer-context-graph-design.md`): memory is a registered graph that renders in the unified Context Graph canvas. Reuses the Discover scope model (`2026-05-28-explore-discover-refactor-design.md`) for pipeline targeting. Modeled on agentcy's "Dreaming pipelines" (`agentcylabs/agentcy`, crate `agentcy-memory` + `pipelines/memory_ingest.rs`).

---

## 1. Goal & non-goals

### Goal

Give **Ask Pensieve** a real, configurable memory:

1. **Sessions** — multi-turn conversations that persist across restarts; follow-ups work; turns are recorded so memory ingestion has something to read.
2. **Agentic Memory** — conversation turns (and other Pensieve data) get consolidated into **memory nodes** that link to existing graph entities via `REFERENCES` edges, forming a "neural network" over the knowledge graph.
3. **Configurable memory pipelines** — defined abstractly against Pensieve tables / graphs / namespaces / connectors (the Discover `Scope` model), with modes for session-turn ingestion, source ingestion, and autonomous "dreaming" (self-learning + consolidation). System-prompt-as-config; an LLM agent with memory tools does the work.
4. **Natural-language recall API** — `/v1/memory/query` with a fast (vector+graph+filter) path for low-latency context injection and an agentic (server agent synthesizes/summarizes) path. Filterable by namespace, connector, type, time, importance, status.
5. **Claude Code integration via hooks** — `pensieve install-hooks` wires SessionStart/UserPromptSubmit → recall/inject and Stop/PostToolUse → cheap capture, so Claude Code automatically deposits conversational context into Pensieve and pulls relevant context back out.
6. **Server-side summarization** of long sessions and of retrieved context; **consolidation/maintenance** of memory nodes (merge, dedup, importance decay, status lifecycle).
7. **A management UI** to view/search/edit memories, configure/monitor pipelines, and see memories on the graph.

The deliverable is **end-to-end and runnable**: a user runs `pensieve install-hooks`, works in Claude Code across sessions, and Pensieve quietly captures turns; later, a `dream` pipeline consolidates them into memory nodes linked to repos/services/tables; the next session's SessionStart hook injects the relevant memories back into Claude Code's context — and the user can see and curate it all in the Memory page.

### Non-goals

- **Per-turn LLM extraction.** Capture is cheap and synchronous; consolidation is batched/scheduled. We never run an LLM on every turn.
- **A bespoke vector store.** We reuse Pensieve columnar storage + the existing `cosine_distance` UDF, not a new pgvector index or a memvid-style file format.
- **A new agent runtime.** The ingestion/dream agent is the existing ADK-Rust runner with memory tools + a config-built prompt.
- **Cross-tenant memory sharing / billing.** Deferred to the cloud track.

---

## 2. Key decisions (locked)

1. **Storage = Pensieve columnar tables.** `memory_nodes` / `memory_edges` are ordinary Pensieve tables (object-store extents) written through the ingest `WritePath` and registered as the `memory` graph via `graph_registrations` → `StoredGraphProvider`. Memory is therefore **first-class queryable** (`run_sql` / `run_kql` / Discover) and renders in the unified GraphView like any other graph. The trade-off — append-only storage — is handled with **latest-wins dedup** (the `ROW_NUMBER()` pattern `StoredGraphProvider` already uses) for mutations, and by moving high-frequency access tracking off the hot path.
2. **Vector recall reuses the platform UDF.** `pensieve-exec::register_vector_udfs` already provides `cosine_distance` / `l2_distance` / `inner_product` (registered into every `SessionContext`, including `run_sql`, Discover, and the agent tools). Embeddings are stored as a `List<Float32>` column; recall is a `run_sql` query: `... ORDER BY cosine_distance(embedding, make_array(...))`. No new index, no UDF to write. (Full-scan today; a perf-index phase is deferred.)
3. **Capture mode = hybrid.** Cheap raw-turn capture is always on; scheduled dreaming is **off by default** until the user configures a pipeline interval or runs it manually.
4. **Namespacing = configurable (both).** Memories carry `database` + `realm`. Default recall = current-project realm (derived from cwd/repo) **∪** a shared `global` realm; `--namespace`/`--connector` filters refine.

### Cross-cutting principles (ported from agentcy)

- **Capture/consolidation split** — cheap synchronous CAPTURE vs. expensive async CONSOLIDATION.
- **Status lifecycle** — `active` / `background` / `archived` (archived hidden from recall).
- **Blended ranking** — recall re-rank = `0.7 * relevance + 0.3 * importance`.
- **Graph linking** — memory → entity via cross-graph `REFERENCES` edges.
- **System-prompt-as-config** — ingestion agent behavior = base + mode-specific + operator instructions.

---

## 3. Architecture overview

```
 Claude Code ── hooks ───────────────────────────────────────────────┐
   SessionStart / UserPromptSubmit → `pensieve memory recall`  (READ)     │
   Stop / PostToolUse             → `pensieve memory ingest-turn` (WRITE) │
                                                                      │ HTTP (Bearer)
 web/ (React)  Memory page · pipeline mgr · GraphView memory layer    │
                                                                      ▼
 pensieve-server   /v1/agent/ask            (SSE, now session-aware)   ┌──────────────┐
               /v1/agent/sessions*      (list/get/turns/delete)    │  AgentState  │
               /v1/memory/events        (cheap capture buffer)     │  + Memory    │
               /v1/memory/query         (fast | agentic recall)    │    handles   │
               /v1/memory/pipelines*    (CRUD · run · status)      └──────┬───────┘
               /v1/memory/items*        (memory CRUD for UI)              │
                       │                                                  │
        ┌──────────────┼───────────────────────────┬─────────────────────┤
        ▼              ▼                           ▼                     ▼
   pensieve-memory     pipeline_runner            RecallService        memory agent tools
   (writer +       (reuses agent runner +     (cosine UDF +        save/recall/list/
    recall over    memory tools + config      filter + 0.7/0.3     status/importance/
    columnar)      prompt; dream scheduler)   rerank)              link/merge
        │              │                                                  │
        ▼              ▼                                                  ▼
   memory_nodes / memory_edges (Pensieve columnar)  ──register──►  "memory" graph
   memory embeddings = List<Float32> column        (graph_registrations →
                                                     StoredGraphProvider → GraphView)
```

---

## 4. Phase decomposition

### M0 — Sessions (activate A5) — *foundation, build first*

Multi-turn persistence on top of the dormant `agent_sessions` / `agent_session_turns` tables.

- **Migration `015_memory_sessions.sql`:** `ALTER TABLE agent_sessions ADD COLUMN title TEXT, rolling_summary TEXT, summary_turn_index INT NOT NULL DEFAULT 0, source TEXT NOT NULL DEFAULT 'pensieve'`; index on `last_active DESC`.
- **`/v1/agent/ask`:** `AskRequest` gains optional `session_id`. The handler creates-or-loads the session, loads prior turns + `rolling_summary`, seeds them into the ADK session via `SessionService::append_event` (each turn an `Event` with `Content::new(role).with_text(...)`, `author` = user/agent), persists the user turn before the run and the assistant turn on `answer_final`, binds the real `session_id` in `persist_run`, and emits a leading SSE `session { session_id }` frame.
- **Rolling summary:** when `turn_count - summary_turn_index > N` (env `PENSIEVE_SESSION_SUMMARY_EVERY`, default 12), a detached task summarizes the conversation and updates `agent_sessions`. Keeps the request path clean and bounds context growth.
- **New endpoints:** `GET /v1/agent/sessions`, `GET /v1/agent/sessions/:id`, `GET /v1/agent/sessions/:id/turns`, `DELETE /v1/agent/sessions/:id`.
- **CLI:** `pensieve query --session <id>` / `--continue` (stores `last_session_id` in `~/.pensieve/config.json`); `pensieve sessions <list|show|turns|delete>`.
- **Caveat:** the `ClaudeCli` engine bypasses ADK; under it `--session` is record-only (no replay). Pipelines and history injection require a non-CLI engine.

### M1 — Memory substrate — *foundation, build first*

The `pensieve-memory` crate + the columnar memory graph + memory agent tools.

- **Crate `pensieve-memory`:** `types.rs` (`MemoryEntry`, `MemoryType{Fact,Decision,Preference,Learning,Summary}`, `MemoryStatus{Active,Background,Archived}`, `CreateMemory`, `MemoryListFilter`, `MemorySearchRequest`, `RecallHit`, `MemoryEdge`); `writer.rs` (`MemoryWriter`: build node/edge rows, embed content via `pensieve-embed`, append via `pensieve-ingest-core::WritePath` group-commit, register the `memory` graph on first write); `recall.rs` (`RecallService`: cosine-UDF query + filter + 0.7/0.3 re-rank + off-hot-path access event); `error.rs`.
- **Columnar tables (auto-provisioned, no Postgres DDL):** `memory_nodes(id, updated_at, database, realm, namespace, content, content_preview, title, memory_type, tags[], importance, status, source_session_id, source_run_id, embedding List<Float32>, created_at)` and `memory_edges(id, src "memory:<uuid>", dst <composite cross-graph id>, type, properties, realm, created_at)`. Append-only; latest-wins dedup by `id`. Registered via `graph_registrations`.
- **Recall** uses the existing `cosine_distance` UDF over the dedup'd latest rows; mutation (importance/status/title/tags) = append a new version; **merge** = append archived versions + a `MERGED_INTO` edge + the consolidated node.
- **Access tracking off the hot path:** recall emits a lightweight `memory_access` event aggregated during dreaming (avoids per-recall append amplification).
- **Agent tools** (registered in `agent/tools.rs`, usable by both Ask Pensieve and the ingestion agent): `save_memory`, `recall_memory`, `list_memories`, `update_memory_status`, `update_memory_importance`, `link_memory_to_entity`, `merge_memories`.
- **Cross-graph edge rendering:** `GraphCanvas.tsx` `edgeEndpointKey` resolves the target endpoint's namespace independently (carried on the edge as `target_namespace`), so a `memory:* → default::github_nodes::repo:*` edge draws in the unified canvas.

### M2 — Capture path (raw event buffer)

- **Postgres table `memory_raw_events(id, tenant_id, session_id?, source, namespace?, event_type, payload JSONB, created_at, processed_at)`** with a partial index on unprocessed rows.
- **`POST /v1/memory/events`** — cheap synchronous insert, no LLM. Requires the `PENSIEVE_TOKEN` bearer (min role Write) so `tenant_id` is populated. This is what Claude Code Stop/PostToolUse hooks write to via `pensieve memory ingest-turn`.
- **`pensieve memory ingest-turn`** reads the hook's stdin JSON (parsed defensively — whole payload stored, fields extracted best-effort) and POSTs it.

### M3 — Memory pipelines (the configurable Agentic Memory layer)

- **Tables:** `memory_pipelines(id, tenant_id, name, mode∈{events,source,dream}, scope_json, instructions, settings_json, schedule_ms?, enabled, status, progress_json, last_run_at, last_success_at, last_error, …)` and `memory_pipeline_runs(…trace_json, counts…)`. Reuse `background_tasks` with `kind='memory_dream'` + a partial-unique index (mirrors connectors).
- **Targeting** is the Discover `Scope` (db.table globs / saved view) extended with `graphs`, `connectors`, `namespaces`.
- **Pipeline runner** reuses `agent::runner` — same engine, tool set = read tools (`run_sql`/`run_kql`/`graph_traverse`/`explore_schema`) + memory tools, instruction built from `mode` + operator `instructions` + `settings`. Tool events fold into `progress_json` (live status) instead of SSE.
- **Scheduler** is a clone of the connector scheduler; enqueues due pipelines (events-mode only when unprocessed events exist). **Dreaming off by default.**
- **Consolidation/housekeeping** (dream mode): recall existing memories, merge dupes, decay importance, aggregate access counts, refresh stale facts against current graph state.
- **Endpoints:** `POST/GET /v1/memory/pipelines`, `GET/PATCH/DELETE /:id`, `POST /:id/run|pause|resume`, `GET /:id/status|runs`. CLI `pensieve memory pipeline <create|list|run|status|…>`.

### M4 — Natural-language recall API

- **`POST /v1/memory/query`** with `mode: fast | agentic` and `filters` (namespace/realm, connector/source, memory_type, time range, importance_min, status).
  - **fast** — `pensieve-memory::recall`: embed query → cosine-UDF ANN → enrich → filter → 0.7/0.3 re-rank → return memories + a compact `context` block. One embed + one query; suitable for hooks.
  - **agentic** — a short-budget agent (reuses runner) with recall + `graph_traverse` + discover tools that synthesizes/summarizes a `brief`.
- Default recall scope = current project realm ∪ `global`. CLI defaults to `fast`; `--agentic` opts into synthesis. `pensieve memory recall|query`.

### M5 — Claude Code hooks integration

- **`pensieve install-hooks`** merges (never clobbers) a `hooks` block into Claude Code `settings.json` and writes `HOOKS.md`. Map: `SessionStart` + `UserPromptSubmit` → `pensieve memory recall … --fast --json` (emits `{"hookSpecificOutput":{"hookEventName":…,"additionalContext":…}}`, exit 0 → injected); `Stop` + `PostToolUse` → `pensieve memory ingest-turn` (`async: true`, capture only). Optional `--transport http` form posts straight to the endpoints.
- Hook events + I/O contract verified against the Claude Code hooks docs (stdin JSON: `session_id`, `transcript_path`, `cwd`, `hook_event_name`, per-event fields; context injection via `hookSpecificOutput.additionalContext` or stdout; exit 2 = blocking).
- **Loop:** SessionStart/UserPromptSubmit → recall → inject; Stop/PostToolUse → ingest-turn → raw buffer → scheduled dream pipeline → structured memories linked to graph → next recall surfaces them.

### M6 — Memory management UI

- A **Memory page** in `web/`: list/search/filter (type/status/tags/namespace/time), detail sheet (edit title/tags, change status/importance), create memory, pipeline manager (create/configure/schedule, live progress, run history), dreaming-interval setting, self-learn trigger.
- **GraphView integration:** the `memory` graph is visible in the unified canvas; a "memories about this entity" panel surfaces `REFERENCES` edges when a graph node is selected. Reuses DiscoverPage / GraphView / Zustand patterns.

### M7 — Hardening (deferred)

Retention/redaction sweep for `memory_raw_events`; an optional pgvector accelerator index for hot realms; a generic `PeriodicScheduler<Kind>` extraction to de-duplicate the connector/memory schedulers; recall re-rank weight tuning.

---

## 5. Data model summary

| Store | Object | Mutability | Why |
|---|---|---|---|
| Pensieve columnar | `memory_nodes`, `memory_edges` | append + latest-wins dedup | first-class queryable, renders in GraphView |
| `memory_nodes.embedding` | `List<Float32>` | versioned with node | recall via existing `cosine_distance` UDF |
| Postgres (control) | `agent_sessions`(+cols), `agent_session_turns` | mutable | session continuity, rolling summary |
| Postgres (control) | `memory_raw_events` | insert + mark processed | cheap capture buffer (hooks) |
| Postgres (control) | `memory_pipelines`, `memory_pipeline_runs` | mutable | pipeline config + live progress |

---

## 6. Verification (per foundation phase)

**M0:** migration applies; `pensieve query "remember X=5" --session s1` then `pensieve query "what is X?" --continue` recalls it; `GET /v1/agent/sessions/:id/turns` returns ordered turns; SSE includes a `session` frame; `cargo test -p pensieve-server`.

**M1:** `cargo test -p pensieve-memory` (writer round-trip, recall re-rank ordering, merge archives sources + repoints edges); `save_memory` via the agent writes a `memory_nodes` row and a node in `GET /v1/graph/memory/overview`; `recall_memory` returns it ranked; a `link_memory_to_entity` to a `github_nodes` repo renders as a cross-graph edge in the unified GraphView; memory is queryable via Discover (`*.memory_nodes`) and `run_sql` with `cosine_distance`; `web` typecheck/build passes after the GraphCanvas/GraphView edge fix.

---

## 7. Risks / open items

- **ClaudeCli engine** can't replay history or run the ingestion agent — sessions degrade to record-only; pipelines/agentic recall require a non-CLI engine. (Decide hard-error vs. silent downgrade.)
- **Append-only write amplification** — keep access tracking off the hot path; rely on Pensieve extent compaction for node-version churn.
- **Embedding dimension** is fixed by the configured backend (fastembed default = 384); a backend swap requires re-embedding. Gate the memory column dim on the active backend.
- **Hook field drift** — parse hook stdin defensively (store whole payload).
- **Scheduler duplication** (M3) — a generic scheduler extraction is an M7 follow-up, not a blocker.
- **Raw-event growth + PII** — needs an M7 retention/redaction sweep before broad rollout.
