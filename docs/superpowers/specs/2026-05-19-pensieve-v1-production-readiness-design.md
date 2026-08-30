# kyma v1.0 Production Readiness — Master Design

**Status:** approved 2026-05-19. Decomposes into 8 sub-specs. Each sub-spec gets its own implementation plan.

**Budget:** 6 months for M1–M3 (post-prerequisite). Total wall-clock to v1.0 ≈ 9+ months including the format-v1 prerequisite (P0).

---

## 1. Goal & non-goals

### Goal

kyma v1.0 is the version we put an OSS-grade stability sticker on. From v1.0 forward, anyone running `kyma-bin` on commodity hardware against an S3-compatible object store and a Postgres instance can ingest production telemetry and query it with:

- **No data-loss surprises.** The five invariants from `docs/architecture.md` hold under crash, Postgres failover, object-store throttling, schema-evolution races, and compaction interleaving. Each invariant is enforced by a CI test.
- **A written contract for what we won't break.** REST + Flight API, KQL dialect, SQL dialect, MCP surface, extent on-disk format, catalog schema, config keys. Breaking changes go through a written deprecation policy.
- **Self-observability sufficient to answer "what is kyma doing right now"** without source-code knowledge — documented metrics taxonomy, structured logs, internal traces, default Grafana dashboards, and per-failure-mode runbooks.
- **A reproducible release.** Signed binaries + container image, SBOM, documented upgrade path, public benchmark harness whose numbers anyone can reproduce.

### Non-goals for v1.0

(Roadmap items that v1.1+ adds without breaking the v1.0 contract.)

- Multi-node read scale-out, ingest scale-out, cross-region federation (slices 2–4 in `architecture.md`).
- Vector / agent-memory column format.
- Full Flight-SQL compliance.
- PromQL frontend.
- New ingest frontends beyond REST + OTLP + Kafka + file-drop.
- Enforced multi-writer concurrency. (v1.0 documents a "one writer per table" rule; enforcement is post-v1.0.)
- Anything that requires breaking the v1.0 stability contract.

---

## 2. Per-axis definition of done

### Axis 1 — Durability & correctness

Each invariant in `docs/architecture.md` has a written failure-mode list and a passing CI test. Required behaviors:

- **Crash-anywhere safe.** Kill `kyma-bin` mid-ingest, mid-query, mid-commit, mid-compaction → restart → no data loss, no torn snapshots, no duplicate extents.
- **Postgres failover safe.** Primary→replica promotion mid-commit → idempotency keys survive, commit either fully lands or fully aborts.
- **Object-store fault safe.** S3 503 storms, transient timeouts → bounded retries, no partial writes ever visible to readers.
- **Schema-evolution safe.** `ALTER TABLE ADD COLUMN` mid-ingest never rewrites history; old extents read with null-fill.
- **Catalog ↔ object-store skew has a documented recovery procedure**, tested by GC reconciliation scenarios.
- **Ingest exactly-once within a documented idempotency window.** Window length to be set by the A1 sub-spec; tested.

### Axis 2 — Stability surface

A single `docs/stability.md` names every frozen surface and the deprecation policy.

**Frozen surfaces for v1.0:**

- HTTP REST API (`/v1/ingest`, `/v1/query`, `/health`, `/metrics`).
- Arrow Flight gRPC action set + ticket format.
- KQL dialect — a documented subset; anything outside is "best-effort, not v1.0 surface."
- SQL dialect — DataFusion's subset minus opt-outs, enumerated.
- **MCP wire surface** (`kyma-mcp`) — frozen per the resolution in §5.
- Extent on-disk format (magic + version byte; v1.x readers read every v1.x writer's extents).
- Catalog Postgres schema — forward-only migrations only; never drop or rename.
- Config keys and env vars.

**Deprecation policy.** Minimum 6-month dual support, structured deprecation warnings, changelog entry.

**Back-compat test matrix.** CI loads extents + catalog snapshots from the last 3 tagged versions and runs a fixed query set against them.

### Axis 3 — Operability

An operator with no kyma source-code knowledge can run kyma, watch it, and debug a failure using only shipped artifacts.

- **Metrics taxonomy.** Every metric named `kyma_<subsystem>_*`, listed in `docs/metrics.md`; same deprecation policy as the stability surface.
- **Structured JSON logs** with a stable field set (`trace_id`, `tenant_id`, `request_id`, `query_id`, etc.) and documented log levels.
- **Internal OTLP tracing.** kyma emits its own spans for commits, queries, compactions, GC. The engine is self-traceable.
- **Default Grafana dashboards** shipped as JSON: ingest throughput / error rate, query p50/p99 by frontend, pruning effectiveness, commit-coordinator queue depth, GC lag.
- **Default Prometheus alert rules** for every failure mode the gauntlet exercises.
- **Runbooks** in `docs/runbooks/`: one per gauntlet scenario. Each runbook documents symptom → dashboard panels → log queries → mitigation → recovery.

### Axis 4 — Test rigor (the gauntlet)

A single `scripts/gauntlet.sh` runs the full battery against a fresh kyma.

- **Property + fuzz** — KQL parser fuzz, extent encoder/decoder roundtrip, catalog migration property tests.
- **Deterministic sim** — injects crashes / pg-failover / object-store-throttling at every point in the ingest and commit paths. One scenario per invariant. Scope is narrow: invariants only, not feature coverage.
- **Chaos** — pg primary kill, MinIO 503 storm, network partitions, clock skew. Extends `scripts/chaos-test.sh`.
- **Soak** — 24h sustained-throughput ingest against real MinIO + pg with periodic queries. Pass = no data loss, no memory leaks, no commit deadlocks.
- **Back-compat replay** — extents from the last 3 tagged versions return identical query results.
- **Perf regression** — published thresholds for ingest/s and query p99 on the reference dataset; regression > threshold fails CI.

**Tier schedule.** PR runs fast tier (property + fuzz + sim subset). Nightly runs full chaos + 1h soak. Weekly runs full 24h soak.

---

## 3. Sub-spec decomposition

Eight sub-specs. Each is independently sized for a single implementation plan (1–4 weeks of focused work) with explicit inputs / outputs so they can be picked up without coordination overhead.

### Phase-0 prerequisite

**P0 — Format v1 completion**
*Scope:* finish the in-flight telemetry format work — Gorilla floats, delta-of-delta timestamps, FST term dicts, full inverted index — and stabilize the extent format under `kyma-format-tlm` enough that F1 can freeze it. Owns its own gauntlet additions for format-level invariants.
*Hard prereq for M1.* All other specs assume the format has landed.

### Foundation specs (M1)

**F1 — Stability Contract**
*Scope:* enumerate every frozen surface, write the deprecation policy, set up back-compat CI.
*Outputs:* `docs/stability.md`, deprecation-policy section of `CONTRIBUTING.md`, CI job replaying the last 3 tagged versions, the shared metrics-taxonomy *rules* that Axis-3 work in area specs follows.

**F2 — Test Gauntlet**
*Scope:* the harness that proves each invariant — property/fuzz + deterministic sim + chaos + soak + back-compat + perf regression.
*Outputs:* `scripts/gauntlet.sh`, deterministic-sim crate or test module, three CI workflow tiers (PR / nightly / weekly), perf thresholds.

### Area specs (M2)

Each follows the same template: current state + known gaps → durability invariants in this area → ops surface → gauntlet scenarios this area must pass.

**A1 — Ingest readiness** — REST, OTLP, Kafka, file-drop, staging buffer, commit coordinator. Idempotency contract, retry/backpressure shape, ALTER TABLE mid-ingest, commit ordering under pg failover. Documents the "one writer per table" deployment rule.

**A2 — Query readiness (incl. MCP)** — KQL frontend, SQL frontend, Flight gRPC, pruning cascade, DataFusion adapter, **MCP surface**. KQL/SQL/MCP dialect freezes, query budgets, cancellation, error taxonomy.

**A3 — Storage + catalog readiness** — `kyma-storage`, `kyma-catalog`, `kyma-format-tlm`, compaction, retention, soft-delete + physical GC. Format freeze (post-P0), catalog schema freeze, GC reconciliation, format version-byte handling.

**A4 — Multi-tenancy + security readiness** — bearer-token auth, role model, tenant isolation (extends `78616f12` tenant-segmented extent paths and Slice 0 retrofit `abc26766`), quotas, TLS everywhere, secret handling, dependency-vuln scanning (cargo-deny, cargo-audit), supply-chain hardening. Multi-tenancy work rebases its gap analysis at A4's start, not at master-spec time.

### Release engineering (M3)

**R1 — Release engineering** — signed release pipeline, SBOM, container hardening, documented upgrade path (across 2 minor versions), public benchmark harness with reproducible numbers, docs polish pass.

---

## 4. Sequencing & milestones

P0, F1, and F2 all start at week 0 and run on parallel tracks. P0 gates only the *format-related clauses* of F1 (the extent-format freeze section of `docs/stability.md`) and the *format-level scenarios* of F2 — most of F1 and F2 can be built without waiting on P0. M2 cannot start until P0, F1, and F2 are all complete.

```
P0 (Format v1) ────────────────────────────┐
                                           │
F1 (stability)  ─── most clauses ──────────┤
                    └── format clauses ────┤
F2 (gauntlet)   ─── most scaffolding ──────┤
                    └── format scenarios ──┴──> A1 (ingest)    ──┐
                                              ├──> A2 (query+MCP) ──┤
                                              ├──> A3 (storage)   ──┼──> R1 ──> v1.0.0
                                              └──> A4 (security)  ──┘
                                              └── [gauntlet grows alongside every area spec]
```

| Phase | Weeks | Outputs |
|---|---|---|
| **P0 — Format v1** | ~12+ (estimated), parallel with M1 | Format work landed and stable; format-level gauntlet scenarios |
| **M1 — Foundations** | weeks 0–6 (parallel with P0); format clauses finalize after P0 lands | F1, F2 skeleton; `v1.0.0-pre.1` tag waits on P0 |
| **M2 — Area readiness** | 14 weeks, starts after both M1 and P0 complete | A1 + A3, A2, A4; gauntlet expanded |
| **M3 — Release eng + RCs** | 6 weeks | R1, ≥ 4 weeks of `v1.0.0-rc.N` tags, signed `v1.0.0` |

**M2 wave ordering:**

- **Wave 1 (weeks 6–14): A1 Ingest + A3 Storage/Catalog.** Tightly coupled — most durability invariants live across the A1↔A3 seam (commit coordinator ↔ catalog CAS ↔ object-store extents). Doing them together lets the gauntlet scenarios written for one validate the other.
- **Wave 2 (weeks 10–18): A2 Query + MCP.** Starts once A1/A3 data-on-disk shape is stable.
- **Wave 3 (weeks 14–20): A4 Multi-tenancy + Security.** Goes last — cross-cutting hardening pass over already-stable subsystems. A4 rebases its gap analysis at start.

Each area spec, on completion, expands the gauntlet with its scenarios and ships its ops surface. Tag at the end of each wave.

**M3 RC discipline.** Minimum 4 weeks of `v1.0.0-rc.N` tags during which the contract is frozen and only fixes land. Full 24h soak runs weekly; any failure resets the soak clock.

---

## 5. Decisions and risks

### Decisions taken (locked at master-spec time)

| Decision | Choice |
|---|---|
| Extent format freeze timing | **Wait for format v1.** P0 prereq added. v1.0 wall-clock extended. |
| MCP in v1.0 stability surface | **In v1.0.** Frozen with the rest of the surface; owned by A2. |
| Supported deployment shape | **Single OR multi-process single-cluster** with documented "one writer per table" rule. No multi-writer enforcement. Operators responsible for not double-writing. |
| Deprecation window | **6 months** dual support. |
| Back-compat test matrix depth | **Last 3 tagged versions.** |
| DataFusion version policy | **Pin for v1.0**, re-evaluate for v1.1. R1 owns documenting the pinned version. |

### Risks to monitor during execution

- **P0 timeline is the long pole.** Format-v1 work is the v1.0 critical path. The "6-month M1–M3 budget" is meaningful only once P0 lands. If P0 itself slips, v1.0 slips one-for-one.
- **Multi-tenancy is a moving target.** Slice 0 (`abc26766`) and tenant-segmented extent paths (`78616f12`) landed recently; the ground will shift again before A4 starts. Mitigation: A4 does its gap analysis at the *start* of its window. The master spec commits to *what* A4 delivers, not *how*.
- **Gauntlet maintenance drift.** Deterministic sim harnesses are expensive to keep in sync. Mitigation: F2 scopes sim narrowly to the five invariants only. Feature regressions stay in `scripts/test-*.sh`.
- **A1↔A3 parallel slip.** Most durability work lives across this seam; if either slips, both wait. No mitigation beyond awareness — keeping them as separate specs makes slip per-spec visible rather than hidden in a merged plan.
- **"One writer per table" rule fragility.** Operators violating the rule corrupt their own data. Mitigation: startup warning in `kyma-bin` if multiple instances detected against the same catalog without role-split env vars; documentation in `docs/operating.md`; alert rule shipped in A4.
- **Public benchmark integrity.** Once published, the numbers can be challenged. Mitigation: R1 owns reproducibility — public dataset, public harness, public hardware spec, reproducible-on-laptop fast tier.

---

## Appendix — Sub-spec stubs

Each stub is the seed for a per-spec brainstorming/writing-plans cycle. They are not implementation plans — they list the work the sub-spec must scope.

### P0 — Format v1 completion

- Gap analysis: what's landed in `kyma-format-tlm` today vs target (Gorilla, delta-of-delta, FST, inverted index).
- Format version byte semantics; reader compatibility rules.
- Format-level gauntlet additions (encoder/decoder roundtrip, mid-format corruption recovery).
- Acceptance criteria for "format frozen, ready for F1."

### F1 — Stability Contract

- Inventory pass over every public surface; classify each as in/out of v1.0.
- Write `docs/stability.md` (the contract) and deprecation-policy section in `CONTRIBUTING.md`.
- Define the metrics-taxonomy *rules* (naming, labels, deprecation procedure) that area specs follow.
- Build back-compat CI job: pull the last 3 tagged versions, replay a fixed query set, fail on divergence.

**Status:** ✅ F1 implementation complete — see `docs/superpowers/plans/2026-05-19-f1-stability-contract.md`. Format clause in `docs/stability.md` section 8 is a placeholder until P0 lands.

### F2 — Test Gauntlet

- Design the deterministic-sim harness (a new test module or `kyma-sim` crate); scope strictly to invariants.
- Map each Axis-1 failure mode to its sim/chaos/soak scenario.
- Build `scripts/gauntlet.sh` and the three CI tier workflows.
- Define perf-regression thresholds on the existing benchmark dataset; document how to update them.

### A1 — Ingest readiness

- Gap analysis: REST, OTLP, Kafka, file-drop, staging buffer, commit coordinator.
- Lock idempotency window and write down the contract.
- Add ingest-path gauntlet scenarios (pg failover mid-commit, object-store 503 storm, ALTER TABLE mid-ingest).
- Define ingest metrics + logs + traces + dashboards + runbooks.
- Document and warn on "one writer per table" deployment rule.

### A2 — Query readiness (incl. MCP)

- Lock KQL, SQL, Flight, MCP dialect/surface for v1.0.
- Define query budgets, cancellation behavior, error taxonomy.
- Add query-path gauntlet scenarios (timeout, cancellation, malformed query, pruning correctness).
- Define query metrics + logs + traces + dashboards + runbooks.

### A3 — Storage + catalog readiness

- Lock catalog Postgres schema; write the forward-only migration policy.
- Lock format (post-P0) and document version-byte semantics for v1.x readers.
- Add storage-path gauntlet scenarios (GC reconciliation, compaction interleaving, soft-delete + physical GC race).
- Define storage/catalog metrics + logs + traces + dashboards + runbooks.

### A4 — Multi-tenancy + security readiness

- Rebase gap analysis on whatever multi-tenancy looks like at A4 start.
- Lock tenant isolation model, quotas, role model.
- TLS everywhere, secret handling, cargo-deny + cargo-audit in CI, supply-chain hardening.
- Add tenant-isolation gauntlet scenarios (cross-tenant query attempt, quota breach, secret exposure).
- Define multi-tenancy + security metrics + logs + traces + dashboards + runbooks.

### R1 — Release engineering

- Signed-release pipeline (cosign-style); container hardening; SBOM generation.
- Documented upgrade path across 2 minor versions; tested in CI.
- Public benchmark harness: reproducible-on-laptop fast tier + full-hardware tier; public dataset; published numbers on `getkyma.dev`.
- Docs polish pass: every page reflects v1.0 surface; quickstart works against the v1.0 binary.
- RC discipline: at least 4 weekly `v1.0.0-rc.N` tags; full 24h soak weekly; any failure resets the clock.
