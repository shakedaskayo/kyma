# Pensieve Memory HITL & Conflict Review — Design

**Status:** approved 2026-06-09. Single implementation slice (one plan, layered execution). Ships behind a default-off master switch, so landing it is a zero-behavior-change event until an operator opts in.

**Relationship to other specs:** Extends the agentic-memory track (`2026-05-31-agentic-memory-design.md`). That design made consolidation (the A.U.D.N. conflict resolver) and dreaming housekeeping (merge / invalidate / archive / entity-link) **fully automatic**, gated only by a `mutation_cap` budget. This design adds an operator-configurable **human-in-the-loop (HITL) policy** over those same mutations, a **review queue**, and a **Review inbox** UI. It reuses the bi-temporal memory model (`crates/pensieve-memory/src/schema.rs`) for cheap, deterministic rollback, and the existing settings surface (`/v1/agent/memory/settings`) for policy storage.

---

## 1. Goal & non-goals

### Goal

Let an operator decide, per deployment, **which automatic memory mutations require human review** — and give them a place to do that review.

1. **HITL policy** — a configurable policy (in existing memory settings) describing, per operation class, whether a mutation runs automatically, applies-then-flags for review, or is held for approval before it touches the live graph.
2. **Uncertainty escalation** — the LLM's own confidence score (currently extracted into `ExtractedMemory.confidence` and then **discarded**) becomes load-bearing: a low-confidence op is escalated one severity level. This is the "candidates we're not sure about" case.
3. **Single enforcement chokepoint** — every memory mutation, whether it originates from real-time consolidation or from a dreaming housekeeping tool call, routes through one classifier. No per-call-site policy logic.
4. **Review queue + inbox** — gated/flagged ops land in a `memory_approval_queue`; a Memory → Review inbox lets a human approve / reject / edit / undo, with the LLM's reasoning, the confidence score, and an old-vs-new diff in front of them.
5. **Deterministic rollback** — because memory is bi-temporal append-only, every gated op has a defined inverse; post-hoc-applied ops carry a precomputed inverse so "undo" is one click.

The deliverable is **end-to-end and runnable**: an operator turns on the policy in settings, marks merges/invalidations as "gate", works normally; the next dreaming run proposes a merge, finds it gated, writes a queue row instead of applying it; the operator opens Memory → Review, sees the two memories side by side with the model's reasoning, edits the merged result, approves it, and the mutation commits.

### Non-goals

- **Approval roles / multi-reviewer workflows.** A single resolver (the current authenticated user). No assignment, no quorum.
- **Notifications.** No email/push. The inbox badge is the only signal for v1.
- **Embeddable-SDK exposure.** The review inbox is a first-party web surface only; `packages/react` exposure is deferred.
- **Gating capture.** Cheap synchronous turn capture is never gated — only consolidation/housekeeping mutations are in scope.
- **A new settings mechanism.** Policy rides on the existing `memory_settings` JSONB row; no new config plumbing.

---

## 2. Key decisions (locked)

1. **Hybrid by risk.** Operations split into severity buckets and each has a configurable base mode: `Auto` (apply, no trace), `PostHoc` (apply + queue row for after-the-fact review/undo), `Gate` (do not apply; queue row held for approval). Defaults: `Invalidate`, `Merge`, `Archive`, `LinkEntityCrossRealm` → **Gate**; `Update`, `RelationshipWrite` → **PostHoc**; `Add` → **Auto**; `PromoteFileCandidate` → **Auto** (high-volume structural candidates; opt-in to gating).
2. **Confidence escalates, it does not replace.** Effective mode = `bump(base_mode, confidence < threshold)` where `bump` is the monotone step `Auto → PostHoc → Gate` (Gate is the ceiling). This gives two independent knobs: "always gate merges" (op-type) **and** "always show me anything the model is unsure about" (confidence). A missing confidence (`None`) is treated as **not** below threshold (no escalation) — we never punish ops the extractor didn't score.
3. **One chokepoint.** A single `policy::classify(...) -> Disposition` is the only place policy is consulted, and a single `gate::dispatch(op, disposition)` is the only place a mutation is either applied, applied-and-logged, or deferred. Every existing mutation site is refactored to call through it.
4. **Rollback via bi-temporal inverse.** No hard deletes. Each op type defines an inverse expressed entirely in existing bi-temporal columns (`valid_at`, `invalid_at`, `superseded_by`, `status`) or edge add/remove. The inverse is computed and stored **at apply time** (for PostHoc) so undo never has to re-derive prior state.
5. **Default off.** `hitl.enabled = false` ships as the default; when false, `classify` returns `Apply` for everything and the system behaves exactly as today. This makes the change safe to land and ship.

---

## 3. Policy model

Added to `MemorySettings` (`crates/pensieve-server/src/agent/memory_settings.rs`), serialized into the same `memory_settings` JSONB row:

```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HitlPolicy {
    pub enabled: bool,                          // master switch; default false
    pub ops: BTreeMap<MemoryOp, OpMode>,        // base mode per op class
    pub confidence_threshold: f32,              // default 0.6; below → escalate one level
    pub realm_scope: Vec<String>,               // empty = all realms
    pub type_scope: Vec<String>,                // empty = all memory types
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpMode { Auto, PostHoc, Gate }

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOp {
    Add, Update, Invalidate, Merge, Archive,
    LinkEntityCrossRealm, RelationshipWrite, PromoteFileCandidate,
}
```

`HitlPolicy::default()` encodes the hybrid-by-risk defaults above and `enabled: false`. `Default` for the whole `MemorySettings` adds `hitl: HitlPolicy::default()`, and deserialization of an existing row missing the `hitl` key falls back to default (so existing tenants are unaffected on upgrade — verified by a serde round-trip test).

### Classification

```rust
pub enum Disposition { Apply, PostHoc, Gate }

pub fn classify(
    op: MemoryOp,
    confidence: Option<f32>,
    realm: &str,
    mem_type: Option<&str>,
    policy: &HitlPolicy,
) -> Disposition
```

Logic:
1. If `!policy.enabled` → `Apply`.
2. If `realm_scope` non-empty and `realm` not in it → `Apply`. Same for `type_scope` / `mem_type`.
3. `base = policy.ops.get(op).copied().unwrap_or(OpMode::Auto)`.
4. `effective = if confidence.is_some_and(|c| c < policy.confidence_threshold) { bump(base) } else { base }`.
5. Map `Auto → Apply`, `PostHoc → PostHoc`, `Gate → Gate`.

This function is pure and exhaustively unit-tested (op × {confidence below/above/none} × base-mode × scope-in/out truth table).

---

## 4. Enforcement chokepoint

Today two subsystems mutate memory:

- **Real-time consolidation** — `crates/pensieve-server/src/agent/memory_conflict.rs`: `consolidate_memory()` → `decide_conflict()` → applies `ConflictOp::{Add, Update, Noop, Invalidate}`.
- **Dreaming housekeeping** — `crates/pensieve-server/src/agent/dreaming.rs` tool handlers: `merge_memories`, `memory_judge` (→ invalidate/supersede), `update_memory_status` (→ archive), `link_memory_to_entity`, `update_memory_importance`.

A new module `crates/pensieve-server/src/agent/memory_gate.rs` introduces:

```rust
pub struct GateOutcome { pub applied: bool, pub queued_id: Option<Uuid> }

pub async fn dispatch(
    state: &AgentState,
    op: MemoryOp,
    ctx: GateCtx,            // realm, mem_type, confidence, source, source_run_id, reason
    payload: OpPayload,      // typed descriptor of the mutation (targets, old/new, edge spec, cosine)
    apply: impl FnOnce() -> BoxFuture<'_, anyhow::Result<AppliedRef>>,
) -> anyhow::Result<GateOutcome>
```

`dispatch` calls `classify`, then:
- **Apply** → run `apply()`, return `{applied:true}`.
- **PostHoc** → run `apply()`, compute the inverse from the returned `AppliedRef` + payload, insert a queue row `{status: auto_applied, mode: post_hoc, inverse}`, return `{applied:true, queued_id}`.
- **Gate** → **skip** `apply()`, insert `{status: pending, mode: gate}`, return `{applied:false, queued_id}`.

Each existing mutation site is refactored so its actual write is wrapped in the `apply` closure and dispatched. The closures return an `AppliedRef` (the ids/versions touched) which is what makes inverse computation deterministic. This is the only structural change to the existing pipelines; their decision logic (which op, what content) is untouched.

**Budget interaction.** Gated (un-applied) ops do **not** consume `mutation_cap` — only real writes do. PostHoc ops consume it (they wrote). Dreaming run telemetry (`decisions_json`) gains `gated` / `post_hoc` / `applied` tallies.

---

## 5. Data model

New migration `crates/pensieve-catalog/migrations/024_memory_approval_queue.sql` (next free number; 022/023 are already doubled up across parallel branches):

```sql
CREATE TABLE memory_approval_queue (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id       UUID NOT NULL,
    realm           TEXT NOT NULL,
    operation       TEXT NOT NULL,    -- MemoryOp serialized
    mode            TEXT NOT NULL,    -- 'gate' | 'post_hoc'
    status          TEXT NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending','approved','rejected','auto_applied','rolled_back')),
    confidence      REAL,
    reason          TEXT,             -- LLM reasoning / conflict explanation
    source          TEXT NOT NULL,    -- 'dreaming' | 'realtime' | 'file_promote'
    source_run_id   UUID,
    payload         JSONB NOT NULL,   -- OpPayload: targets, old/new content, edge spec, cosine, mem_type
    inverse         JSONB,            -- precomputed undo (post_hoc rows); null for gate (nothing applied)
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at     TIMESTAMPTZ,
    resolved_by     TEXT,
    resolution_comment TEXT
);
CREATE INDEX idx_maq_tenant_status ON memory_approval_queue (tenant_id, status, created_at DESC);
CREATE INDEX idx_maq_run ON memory_approval_queue (source_run_id) WHERE source_run_id IS NOT NULL;
```

The catalog row lives in Postgres (like `memory_settings` / `memory_pipeline_runs`) — it is operational metadata, not memory content, so it does not go through the columnar write path. `confidence` is additionally persisted into the memory node's `provenance` JSON when the op applies, closing the "extracted-then-discarded" gap.

---

## 6. Apply / reject / undo semantics

Resolution operations live in `memory_gate.rs` alongside `dispatch`:

- **Approve (gate row):** execute the deferred mutation now from `payload` (re-running the same write the closure would have), set `status=approved`, `resolved_at/by`. Editable: an approve request may carry an edited `payload` (e.g. corrected merged content) which is used instead.
- **Reject (gate row):** set `status=rejected`. Nothing was applied; nothing to undo.
- **Undo (post_hoc row):** execute the stored `inverse`, set `status=rolled_back`. Inverses by op:
  - `Invalidate` → clear `invalid_at` + `superseded_by` on the old node; re-invalidate / drop the superseding node per payload.
  - `Merge` → re-validate the absorbed node (`invalid_at=NULL`, `status=active`); revert the surviving node's content to its pre-merge version (stored in `inverse`).
  - `Archive` → `status=active`.
  - `RelationshipWrite` / `LinkEntityCrossRealm` → remove the added edge (append a tombstone row per the latest-wins edge model).
  - `Update` → restore prior content/version from `inverse`.
- **Bulk:** approve/reject/undo accept a list of ids and apply the single-item path per id, collecting per-id results.

Every inverse is covered by a round-trip unit test: apply op → assert state → run inverse → assert state equals pre-op snapshot.

---

## 7. API

Extend the existing memory router (`crates/pensieve-server/src/agent/routes.rs`). Settings get the new block in-place; the queue gets a small REST surface:

- `GET /v1/agent/memory/settings` / `PUT …` — now include `hitl` in the payload (no new route).
- `GET /v1/agent/memory/review?status=&source=&realm=&op=&limit=&cursor=` — list queue rows (newest first).
- `GET /v1/agent/memory/review/count` — `{ pending: n, post_hoc: n }` for the nav badge.
- `POST /v1/agent/memory/review/{id}/approve` — body optional `{ payload }` (edited).
- `POST /v1/agent/memory/review/{id}/reject` — body optional `{ comment }`.
- `POST /v1/agent/memory/review/{id}/undo` — post_hoc rows only.
- `POST /v1/agent/memory/review/bulk` — `{ ids, action: approve|reject|undo, comment? }`.

All scoped by tenant via the existing auth extractor. Typed into `packages/client/src/memory.ts` (`listReview`, `reviewCount`, `approve`, `reject`, `undo`, `bulkReview`) with shared TS types mirroring the Rust enums.

---

## 8. Web UI

- **New route** `web/src/routes/_app.memory.review.tsx` (+ index) → `ReviewInbox` feature component in `web/src/features/memory/review/`.
  - `ReviewInbox` — filter bar (status / source / realm / op / confidence), list, bulk-select, empty state.
  - `CandidateCard` — op badge + severity color, `confidence` chip, `source` + run deep-link, reason text, **old⇄new diff** (two-column for merge/invalidate/update; single for add), action row: `Approve` / `Reject` / `Edit` / `Open in graph`; `Undo` for post_hoc rows. Edit opens an inline editor on the new-content field before approving.
- **Nav badge** — pending count on the Memory nav entry, polled via `reviewCount` (reuses existing query-client patterns).
- **Settings section** — `MemorySettingsPanel.tsx` gains an "Approval policy" block: master toggle, per-op mode selector (`Auto`/`PostHoc`/`Gate` segmented control), confidence-threshold slider, realm/type scope multi-selects. Hidden detail (per-op rows) collapses when master toggle is off.
- **Run deep-link** — `RunDetail.tsx` shows a "Candidates (n) →" link routing to the Review inbox pre-filtered by `source_run_id`.

The inbox is the canonical worklist; the graph and run views only deep-link into it.

---

## 9. Testing

- **Unit (Rust):** `classify` truth table; `bump` monotonicity; serde round-trip incl. legacy row without `hitl`; one apply→inverse→assert round-trip per op type.
- **Integration (Rust):** for each mode — Gate (not applied + pending row), PostHoc (applied + auto_applied row + inverse present), Auto (applied, no row); approve-applies; reject-no-op; undo-reverses; bulk. Confidence escalation end-to-end (sub-threshold Add becomes a PostHoc row). Default-off → identical to pre-change behavior.
- **API (Rust):** each route happy-path + tenant isolation (cannot resolve another tenant's row) + edited-approve.
- **Web:** `ReviewInbox` / `CandidateCard` render + action wiring (Vitest/RTL, matching existing `*.test.tsx` patterns); settings panel toggle gating.
- **Full build + lint gate before merge:** `cargo build`, `cargo test`, `cargo clippy`, and the web build/test/lint, all green.

---

## 10. File-by-file change surface

**Backend (Rust)**
- `crates/pensieve-server/src/agent/memory_settings.rs` — add `HitlPolicy` / `OpMode` / `MemoryOp` + defaults.
- `crates/pensieve-server/src/agent/memory_gate.rs` — **new**: `classify`, `Disposition`, `dispatch`, resolve (approve/reject/undo/bulk), inverse computation, queue CRUD.
- `crates/pensieve-server/src/agent/memory_conflict.rs` — route A.U.D.N. apply through `dispatch`.
- `crates/pensieve-server/src/agent/dreaming.rs` — route housekeeping tool writes through `dispatch`; add disposition tallies to `decisions_json`.
- `crates/pensieve-server/src/agent/memory_extract.rs` — thread `confidence` into `GateCtx` / `provenance`.
- `crates/pensieve-server/src/agent/routes.rs` — review routes; settings payload already carries `hitl`.
- `crates/pensieve-catalog/migrations/024_memory_approval_queue.sql` — **new** table + indexes.

**Client + Web (TS)**
- `packages/client/src/memory.ts` — review API + types.
- `web/src/routes/_app.memory.review.tsx` (+ index) — **new** route.
- `web/src/features/memory/review/{ReviewInbox,CandidateCard}.tsx` (+ tests) — **new**.
- `web/src/features/agent/MemorySettingsPanel.tsx` — approval-policy section.
- `web/src/features/dreaming/RunDetail.tsx` — candidates deep-link.
- Memory nav component — pending badge.

---

## 11. Rollout

Ships dormant (`enabled:false`). An operator opts in via settings. No data migration risk (additive table + additive JSON key). Reverting the feature is safe: with the policy off, the chokepoint is a pass-through, and any queued rows are inert.
