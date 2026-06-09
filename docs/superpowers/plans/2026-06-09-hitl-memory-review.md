# Memory HITL & Conflict Review — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline, coupled backend) for Tasks 1–6 and superpowers:subagent-driven-development for the independent frontend Tasks 7–11. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add an operator-configurable human-in-the-loop policy over automatic memory mutations (realtime consolidation + dreaming housekeeping), with an approval queue and a Review inbox UI, shipping default-off.

**Architecture:** One pure `classify()` + one `dispatch()` chokepoint decides per mutation whether to apply, apply-and-log (post-hoc), or defer (gate) to a `memory_approval_queue`. Queue persists to Postgres (server) or a JSON file (local mode). Bi-temporal columns make every op's inverse deterministic for rollback. Default-off master switch ⇒ pass-through, zero behavior change.

**Tech Stack:** Rust (axum, sqlx, serde, tokio), the existing `MemoryWriter`/columnar memory store, ADK `FunctionTool`s; React + TanStack Router/Query, Tailwind, Vitest/RTL.

**Spec:** `docs/superpowers/specs/2026-06-09-hitl-memory-review-design.md`

**Note on granularity:** This plan is executed by the same agent that authored it. Load-bearing code (types, `classify`, `dispatch`, inverses, SQL, the tool-wrapping pattern, route + component contracts) is spelled out; mechanical boilerplate references the exact pattern file to copy. Every task ends green + committed.

---

## File structure

**Backend (Rust, `crates/kyma-server/src/agent/`)**
- `memory_policy.rs` — **new**: `HitlPolicy`, `OpMode`, `MemoryOp`, `Disposition`, `classify()`, `bump()`. Pure, no I/O. (~150 lines)
- `memory_queue_store.rs` — **new**: `QueueRow`, `QueueStore` enum (Pg | Local-file), insert/list/get/count/update. (~250 lines)
- `memory_gate.rs` — **new**: `GateCtx`, `OpPayload`, `Inverse`, `dispatch()`, `apply_op()`, `inverse_of()`, `resolve()` (approve/reject/undo/bulk). (~400 lines)
- `memory_settings.rs` — **modify**: add `hitl: HitlPolicy` field + default.
- `memory_conflict.rs` — **modify**: route A.U.D.N. apply through `dispatch`.
- `memory_tools.rs` — **modify**: gate the 5 mutating dreaming tools when `SharedToolCtx.hitl` is set.
- `tools.rs` — **modify**: add `hitl: Option<Arc<HitlGate>>` to `SharedToolCtx`.
- `dreaming.rs` — **modify**: build the `HitlGate` from settings+state; attach to the dreaming `SharedToolCtx`; add disposition tallies.
- `routes.rs` — **modify**: 6 review routes; settings payload already carries `hitl` via the struct.
- `crates/kyma-catalog/migrations/024_memory_approval_queue.sql` — **new**.

**Frontend (`web/src/`)**
- `sdk/review.ts` — **new**: types + `listReview`/`reviewCount`/`approve`/`reject`/`undo`/`bulkReview`.
- `sdk/memory.ts` — **modify**: extend `MemorySettings` TS type with `hitl`.
- `features/review/useReview.ts` — **new**: query/mutation hooks.
- `features/review/ReviewInbox.tsx` — **new**.
- `features/review/CandidateCard.tsx` — **new**.
- `features/review/ReviewInbox.test.tsx`, `CandidateCard.test.tsx` — **new**.
- `routes/_app.memory.review.tsx`, `_app.memory.review.index.tsx` — **new**.
- `features/memory/MemoryHeader.tsx` — **modify**: Review tab + pending badge.
- `features/agent/MemorySettingsPanel.tsx` — **modify**: Approval-policy section.
- `features/dreaming/RunDetail.tsx` — **modify**: "Candidates →" deep-link.

---

## Task 1: Policy types + `classify` (pure core)

**Files:**
- Create: `crates/kyma-server/src/agent/memory_policy.rs`
- Modify: `crates/kyma-server/src/agent/mod.rs` (add `pub mod memory_policy;`)

- [ ] **Step 1: Write the module with types + classify + failing tests**

```rust
//! HITL policy model + the single pure classifier. No I/O — fully unit-tested.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOp {
    Add, Update, Invalidate, Merge, Archive,
    LinkEntityCrossRealm, RelationshipWrite, PromoteFileCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpMode { Auto, PostHoc, Gate }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition { Apply, PostHoc, Gate }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HitlPolicy {
    pub enabled: bool,
    pub ops: BTreeMap<MemoryOp, OpMode>,
    pub confidence_threshold: f32,
    pub realm_scope: Vec<String>,
    pub type_scope: Vec<String>,
}

impl Default for HitlPolicy {
    fn default() -> Self {
        use MemoryOp::*;
        use OpMode::*;
        let ops = BTreeMap::from([
            (Add, Auto), (Update, PostHoc), (Invalidate, Gate), (Merge, Gate),
            (Archive, Gate), (LinkEntityCrossRealm, Gate), (RelationshipWrite, PostHoc),
            (PromoteFileCandidate, Auto),
        ]);
        Self { enabled: false, ops, confidence_threshold: 0.6, realm_scope: vec![], type_scope: vec![] }
    }
}

/// Escalate one severity level when the model is unsure. Gate is the ceiling.
fn bump(m: OpMode) -> OpMode {
    match m { OpMode::Auto => OpMode::PostHoc, OpMode::PostHoc => OpMode::Gate, OpMode::Gate => OpMode::Gate }
}

/// The only place policy is consulted. Pure.
pub fn classify(
    op: MemoryOp, confidence: Option<f32>, realm: &str, mem_type: Option<&str>, p: &HitlPolicy,
) -> Disposition {
    if !p.enabled { return Disposition::Apply; }
    if !p.realm_scope.is_empty() && !p.realm_scope.iter().any(|r| r == realm) { return Disposition::Apply; }
    if !p.type_scope.is_empty() {
        match mem_type { Some(t) if p.type_scope.iter().any(|x| x == t) => {}, _ => return Disposition::Apply }
    }
    let base = p.ops.get(&op).copied().unwrap_or(OpMode::Auto);
    let eff = if confidence.is_some_and(|c| c < p.confidence_threshold) { bump(base) } else { base };
    match eff { OpMode::Auto => Disposition::Apply, OpMode::PostHoc => Disposition::PostHoc, OpMode::Gate => Disposition::Gate }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pol() -> HitlPolicy { let mut p = HitlPolicy::default(); p.enabled = true; p }

    #[test] fn disabled_is_passthrough() {
        let p = HitlPolicy::default(); // enabled:false
        assert_eq!(classify(MemoryOp::Merge, Some(0.1), "r", None, &p), Disposition::Apply);
    }
    #[test] fn gate_op_is_gated() {
        assert_eq!(classify(MemoryOp::Merge, None, "r", None, &pol()), Disposition::Gate);
    }
    #[test] fn auto_add_high_conf_applies() {
        assert_eq!(classify(MemoryOp::Add, Some(0.9), "r", None, &pol()), Disposition::Apply);
    }
    #[test] fn low_conf_escalates_add_to_posthoc() {
        assert_eq!(classify(MemoryOp::Add, Some(0.3), "r", None, &pol()), Disposition::PostHoc);
    }
    #[test] fn low_conf_escalates_posthoc_to_gate() {
        assert_eq!(classify(MemoryOp::Update, Some(0.3), "r", None, &pol()), Disposition::Gate);
    }
    #[test] fn missing_conf_does_not_escalate() {
        assert_eq!(classify(MemoryOp::Add, None, "r", None, &pol()), Disposition::Apply);
    }
    #[test] fn realm_scope_excludes() {
        let mut p = pol(); p.realm_scope = vec!["only".into()];
        assert_eq!(classify(MemoryOp::Merge, None, "other", None, &p), Disposition::Apply);
        assert_eq!(classify(MemoryOp::Merge, None, "only", None, &p), Disposition::Gate);
    }
    #[test] fn type_scope_excludes() {
        let mut p = pol(); p.type_scope = vec!["decision".into()];
        assert_eq!(classify(MemoryOp::Merge, None, "r", Some("fact"), &p), Disposition::Apply);
        assert_eq!(classify(MemoryOp::Merge, None, "r", Some("decision"), &p), Disposition::Gate);
    }
}
```

- [ ] **Step 2: Run tests, expect FAIL (module not wired)** → add `pub mod memory_policy;` to `mod.rs`.
- [ ] **Step 3: Run `cargo test -p kyma-server memory_policy::` → expect PASS (8 tests).**
- [ ] **Step 4: Commit** `feat(memory): HITL policy model + pure classifier`.

---

## Task 2: Migration + queue store (pg + local backends)

**Files:**
- Create: `crates/kyma-catalog/migrations/024_memory_approval_queue.sql`
- Create: `crates/kyma-server/src/agent/memory_queue_store.rs` (+ `mod.rs` line)

- [ ] **Step 1: Write the migration** (table + indexes exactly as in spec §5). Confirm migrations are applied by the existing runner (same dir as `020_memory_settings.sql`); no code wiring needed beyond the file.

- [ ] **Step 2: Write `QueueRow` + `QueueStore`.** Key shape:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueRow {
    pub id: Uuid,
    pub realm: String,
    pub operation: MemoryOp,
    pub mode: String,            // "gate" | "post_hoc"
    pub status: String,          // pending|approved|rejected|auto_applied|rolled_back
    pub confidence: Option<f32>,
    pub reason: Option<String>,
    pub source: String,          // dreaming|realtime|file_promote
    pub source_run_id: Option<Uuid>,
    pub payload: Value,
    pub inverse: Option<Value>,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub resolved_by: Option<String>,
    pub resolution_comment: Option<String>,
}

/// Server mode → Postgres; local mode → a JSON file alongside memory settings.
pub enum QueueStore { Pg { pool: PgPool, tenant: TenantId }, Local { path: PathBuf } }

impl QueueStore {
    pub fn from_state(state: &AgentState) -> Option<Self> { /* pool → Pg; else memory_settings_path.parent()/memory_approval_queue.json → Local; else None */ }
    pub async fn insert(&self, row: &QueueRow) -> anyhow::Result<()>;
    pub async fn list(&self, f: &QueueFilter) -> anyhow::Result<Vec<QueueRow>>; // newest first
    pub async fn get(&self, id: Uuid) -> anyhow::Result<Option<QueueRow>>;
    pub async fn counts(&self) -> anyhow::Result<(i64,i64)>; // (pending, post_hoc auto_applied)
    pub async fn update_status(&self, id: Uuid, status: &str, by: Option<&str>, comment: Option<&str>) -> anyhow::Result<()>;
}
```
Local backend stores `Vec<QueueRow>` as pretty JSON (read-modify-write under a `tokio::sync::Mutex` held in a process-wide `OnceCell` keyed by path, to avoid lost updates). Pg backend uses sqlx with `tenant_id` scoping on every query.

- [ ] **Step 3: Tests.** `cargo test -p kyma-server memory_queue_store::` — local-backend round-trip: insert 2 → list returns newest-first → get by id → update_status → counts reflect it. (Pg path covered later by integration tests in CI with a database; the local path is the unit-tested one.)
- [ ] **Step 4: Commit** `feat(memory): approval-queue table + dual-backend store`.

---

## Task 3: Gate dispatch + inverses + resolve

**Files:**
- Create: `crates/kyma-server/src/agent/memory_gate.rs` (+ `mod.rs` line)

- [ ] **Step 1: Define `GateCtx`, `OpPayload`, `HitlGate`, `dispatch`.**

```rust
pub struct HitlGate { pub policy: HitlPolicy, pub store: Arc<QueueStore>, pub resolver: Option<String> }

pub struct GateCtx<'a> {
    pub op: MemoryOp, pub realm: &'a str, pub mem_type: Option<&'a str>,
    pub confidence: Option<f32>, pub source: &'a str, pub source_run_id: Option<Uuid>,
    pub reason: Option<String>,
}

/// `apply` returns the AppliedRef (ids/versions touched) used to build the inverse.
pub async fn dispatch<F, Fut>(gate: &HitlGate, ctx: GateCtx<'_>, payload: OpPayload, apply: F)
  -> anyhow::Result<GateOutcome>
where F: FnOnce() -> Fut, Fut: Future<Output = anyhow::Result<AppliedRef>>;
```

`dispatch` calls `classify(ctx.op, ctx.confidence, ctx.realm, ctx.mem_type, &gate.policy)`:
- `Apply` → `apply().await?`; `GateOutcome { applied:true, queued_id:None }`.
- `PostHoc` → `let r = apply().await?`; build `inverse_of(op, &payload, &r)`; insert row `{mode:"post_hoc", status:"auto_applied", inverse:Some}`; `applied:true`.
- `Gate` → insert row `{mode:"gate", status:"pending", inverse:None}`; **no apply**; `applied:false`.

- [ ] **Step 2: Implement `apply_op` + `inverse_of` + `resolve`.** `apply_op(store_ctx, op, payload)` performs the actual mutation from a payload (used by both the gate's PostHoc path indirectly and by approve). `inverse_of` returns an `Inverse` JSON per op (spec §6). `resolve(gate, id, action, edited_payload, comment)`:
  - `approve` (gate row): `apply_op(..., edited_payload.unwrap_or(row.payload))`; `update_status(approved)`.
  - `reject`: `update_status(rejected)`.
  - `undo` (post_hoc row): `apply_inverse(row.inverse)`; `update_status(rolled_back)`.

- [ ] **Step 3: Round-trip inverse tests (local store + an in-memory MemoryWriter against a tempdir catalog).** For each op {Add, Update, Invalidate, Merge, Archive, RelationshipWrite}: apply → snapshot latest node/edge state → run inverse → assert equals pre-op snapshot. Use the existing test catalog harness (mirror `memory_settings.rs` test style + `MemoryWriter` test setup found in `crates/kyma-memory` tests).
- [ ] **Step 4: Run `cargo test -p kyma-server memory_gate::` → PASS.**
- [ ] **Step 5: Commit** `feat(memory): gate dispatch + deterministic bi-temporal inverses`.

---

## Task 4: Wire realtime consolidation through the gate

**Files:**
- Modify: `crates/kyma-server/src/agent/memory_conflict.rs` (`consolidate_memory`)
- Modify: `crates/kyma-server/src/agent/memory_settings.rs` (add `hitl` field — do here so consolidation can read it)

- [ ] **Step 1:** Add `hitl: HitlPolicy` to `MemorySettings` + `Default` (defaults to `HitlPolicy::default()`). Serde-roundtrip test incl. a legacy JSON value missing `hitl` → loads default.
- [ ] **Step 2:** In `consolidate_memory`, after `decision`, map `ConflictOp → MemoryOp` (Add→Add, Update→Update, Invalidate→Invalidate; Noop→skip). Build a `HitlGate` from `memory_settings::load_for(state)` + `QueueStore::from_state(state)`. Wrap each apply arm in `gate::dispatch` with `confidence: m.confidence`, `source:"realtime"`, payload carrying the candidate + `decision`. When `dispatch` returns `applied:false`, do **not** bump the tally's written count (it was deferred); add a `gated` counter to `ConflictTally`.
- [ ] **Step 3:** Tests: with `hitl.enabled=false`, behavior identical to today (existing consolidation tests still pass). With enabled + Merge/Invalidate gated, assert the candidate is queued and not written (query the store + assert node absent). 
- [ ] **Step 4: Commit** `feat(memory): gate realtime consolidation (A.U.D.N.)`.

---

## Task 5: Wire dreaming housekeeping tools through the gate

**Files:**
- Modify: `crates/kyma-server/src/agent/tools.rs` (`SharedToolCtx` + `hitl: Option<Arc<HitlGate>>`)
- Modify: `crates/kyma-server/src/agent/memory_tools.rs` (5 mutating tools)
- Modify: `crates/kyma-server/src/agent/dreaming.rs` (build gate, attach to dreaming `SharedToolCtx`, tallies)

- [ ] **Step 1:** Add `pub hitl: Option<Arc<HitlGate>>` to `SharedToolCtx`; default `None` at every existing construction site (interactive agent stays ungated — it is already human-driven). A small helper `gate_or_apply(shared, ctx, payload, apply)` lives in `memory_gate.rs`: if `shared.hitl` is `Some`, `dispatch`; else just `apply()`.
- [ ] **Step 2:** Wrap the mutation in each of: `tool_merge_memories` (op `Merge`), `tool_memory_judge` (`supersedes`→`Invalidate`, `merged`→`Merge`, else `RelationshipWrite`), `tool_update_memory_status` (status=archived → `Archive`; else ungated), `tool_link_memory_to_entity` (cross-realm/cross-namespace → `LinkEntityCrossRealm`, else `RelationshipWrite`), `tool_update_memory_importance` (`Update`). When gated/deferred, the tool returns `{"queued_for_review": true, "queue_id": "...", "applied": false}` so the dreaming agent knows it proposed, not committed.
- [ ] **Step 3:** In `dreaming.rs run_via_adk`, build `HitlGate` from `settings.hitl` + `QueueStore::from_state(state)` and set `shared.hitl = Some(Arc::new(gate))` (only here). Add `gated`/`post_hoc` tallies into `decisions_json` via `observe_tool_call` (read the tool result's `queued_for_review`).
- [ ] **Step 4:** Tests: a dreaming `SharedToolCtx` with an enabled gate → `tool_merge_memories` returns `queued_for_review` and writes a pending row, no archive applied; with gate `None` → applies as today (existing behavior).
- [ ] **Step 5: Commit** `feat(memory): gate dreaming housekeeping tools`.

---

## Task 6: API — settings payload + review routes

**Files:**
- Modify: `crates/kyma-server/src/agent/routes.rs`

- [ ] **Step 1:** Confirm `get/put_memory_settings` already serialize the whole `MemorySettings` (so `hitl` rides along automatically — verify the handler doesn't field-pick). Add a test asserting a PUT with `hitl` round-trips through GET.
- [ ] **Step 2:** Add routes under the existing memory group:
  - `GET /memory/review` → `list` (query: status, source, realm, op, limit, cursor) → `{ items: [...] }`.
  - `GET /memory/review/count` → `{ pending, post_hoc }`.
  - `POST /memory/review/:id/approve` (body optional `{payload}`).
  - `POST /memory/review/:id/reject` (body optional `{comment}`).
  - `POST /memory/review/:id/undo`.
  - `POST /memory/review/bulk` (`{ids, action, comment?}`).
  Each builds `HitlGate` from state, calls `memory_gate::resolve` / `store.list`/`counts`. Write role required for approve/reject/undo/bulk (mirror `import_memory_handler`'s role check); read role for list/count.
- [ ] **Step 3:** Tests: route-level happy paths against a local store (no pool) — list empty, insert via store then list returns it, approve transitions status, tenant isolation on the pg path is asserted in the store tests.
- [ ] **Step 4: Commit** `feat(api): memory review queue + HITL settings endpoints`.

---

## Task 7: Web SDK — `review.ts` + settings type

**Files:**
- Create: `web/src/sdk/review.ts`
- Modify: `web/src/sdk/memory.ts` (extend `MemorySettings` with `hitl`)

- [ ] **Step 1:** Mirror `web/src/sdk/dreaming.ts` exactly (`base`, `headers`, `handleResponse`, `BaseArgs`). Define TS types: `MemoryOp`, `OpMode`, `Disposition`, `HitlPolicy`, `ReviewItem` (= `QueueRow` shape), `ReviewCounts`. Methods: `listReview`, `reviewCount`, `approveReview`, `rejectReview`, `undoReview`, `bulkReview`.
- [ ] **Step 2:** Extend `MemorySettings` TS interface in `sdk/memory.ts` with `hitl: HitlPolicy` (import the type from `review.ts`).
- [ ] **Step 3:** Typecheck: `cd web && npx tsc --noEmit` (or the repo's `pnpm -C web typecheck`) — PASS.
- [ ] **Step 4: Commit** `feat(web): review SDK client + HITL settings type`.

---

## Task 8: `useReview` hooks

**Files:** Create `web/src/features/review/useReview.ts`

- [ ] **Step 1:** Mirror `web/src/features/dreaming/useDreaming.ts`: `useReviewItems(filter)`, `useReviewCount()` (refetchInterval 15s), `useApprove()/useReject()/useUndo()/useBulk()` mutations that invalidate `["review", ...]` keys + toast on success/error.
- [ ] **Step 2:** Typecheck PASS. **Commit** `feat(web): review query/mutation hooks`.

---

## Task 9: ReviewInbox + CandidateCard (+ tests)

**Files:** Create `ReviewInbox.tsx`, `CandidateCard.tsx`, `ReviewInbox.test.tsx`, `CandidateCard.test.tsx` under `web/src/features/review/`; routes `_app.memory.review.tsx` + `.index.tsx`.

- [ ] **Step 1 (TDD):** Write `CandidateCard.test.tsx`: renders op badge + confidence + reason; shows old⇄new for a merge payload; clicking Approve calls the mutation; `Undo` shows only for `post_hoc` rows. Mock the hooks/fetch per `KymaDiscover.test.tsx`.
- [ ] **Step 2:** Implement `CandidateCard` using `@/components/ui` primitives (Card, Badge, Button) + `cn()`, matching the ASCII mock (op badge w/ severity color, confidence chip, source + run deep-link, two-column diff for merge/invalidate/update, action row; inline edit on new-content before approve).
- [ ] **Step 3:** Implement `ReviewInbox` (filter bar: status/source/realm/op/confidence; list of `CandidateCard`; bulk-select + bulk bar; `EmptyState` when none; `SkeletonRows` while loading). `ReviewInbox.test.tsx`: renders a list from mocked `useReviewItems`, empty state when none.
- [ ] **Step 4:** Add the two route files (pattern from subagent report §1).
- [ ] **Step 5:** Run `pnpm -C web test review` → PASS; typecheck PASS. **Commit** `feat(web): review inbox + candidate card`.

---

## Task 10: Settings — Approval-policy section

**Files:** Modify `web/src/features/agent/MemorySettingsPanel.tsx`

- [ ] **Step 1:** Add a `Section title="Approval policy"` after the Dreaming section using existing `Section`/`Row`/`Toggle`/`SliderRow`/`StringSelectRow` helpers: master `Toggle` (`hitl.enabled`); when on, render a per-op row group (each `MemoryOp` → a 3-way segmented control Auto/PostHoc/Gate writing `hitl.ops[op]`), a `SliderRow` for `confidence_threshold` (0–1, step .05), and comma-input rows for `realm_scope`/`type_scope`. Wire through the existing `set('hitl', ...)` + `save()` flow.
- [ ] **Step 2:** Typecheck PASS; manual: toggling off hides the detail rows. **Commit** `feat(web): HITL approval-policy settings UI`.

---

## Task 11: Nav badge + RunDetail deep-link

**Files:** Modify `web/src/features/memory/MemoryHeader.tsx`, `web/src/features/dreaming/RunDetail.tsx`

- [ ] **Step 1:** Add a `{ to: "/memory/review", label: "Review", icon: ShieldCheck, match: "/memory/review" }` tab; render a `Badge` with `useReviewCount().data?.pending` when > 0 (pattern from subagent report §2).
- [ ] **Step 2:** In `RunDetail`, add a "Candidates (n) →" `Link` to `/memory/review?source_run_id=<id>` when the run produced queue rows (use `useReviewCount` filtered by run, or read from run stats `gated`).
- [ ] **Step 3:** Typecheck PASS. **Commit** `feat(web): review nav badge + run deep-link`.

---

## Task 12: Full verification + route tree + merge

- [ ] **Step 1:** Regenerate the TanStack route tree (`pnpm -C web build` or the route-gen step) so `routeTree.gen.ts` includes the new routes. Commit the regen if it changes.
- [ ] **Step 2:** Backend gate: `cargo build -p kyma-server` and `cargo test -p kyma-server` → all green; `cargo clippy -p kyma-server --all-targets -- -D warnings` → clean (fix any).
- [ ] **Step 3:** Frontend gate: `pnpm -C web typecheck`, `pnpm -C web test`, `pnpm -C web build` (or `lint`) → all green.
- [ ] **Step 4:** Manual smoke (document results): start the server in local mode, enable the policy via PUT settings, run a manual dreaming run that proposes a merge, GET `/memory/review` shows the pending row, POST approve applies it. Record actual output.
- [ ] **Step 5: Merge.** `git checkout main && git merge --no-ff worktree-memory-hitl-review` (per the local-merge-sync preference), then push. Exit the worktree (keep or remove).

---

## Self-review checklist (run after writing)

1. **Spec coverage:** policy model (T1), confidence (T1/T4/T5), chokepoint (T3–T5), queue+rollback (T2/T3), API (T6), inbox+settings+badge+deeplink (T7–T11), default-off + local mode (T1/T2/T4), tests + build gate (all + T12). ✓
2. **Placeholders:** none — migration number fixed (024), types/signatures concrete.
3. **Type consistency:** `MemoryOp`/`OpMode`/`Disposition`/`HitlPolicy`/`HitlGate`/`QueueRow`/`QueueStore`/`GateCtx`/`OpPayload` names used identically across T1–T11.
