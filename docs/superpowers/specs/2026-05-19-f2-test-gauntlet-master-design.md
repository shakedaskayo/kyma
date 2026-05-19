# F2 — Test Gauntlet Master Design

**Status:** approved 2026-05-19. Decomposes into 5 sub-specs (F2.1–F2.5). Each sub-spec gets its own brainstorm → plan → execution cycle.

**Parent:** v1.0 master spec at `docs/superpowers/specs/2026-05-19-kyma-v1-production-readiness-design.md`. F2 is the "Test Gauntlet" foundation sub-spec from that document.

**Budget:** ~3–5 weeks if F2.2–F2.5 parallelize; ~6–9 weeks if serial. Fits inside the v1.0 M1 budget alongside F1 (already merged).

---

## 1. Goal & non-goals

### Goal

F2 produces the gauntlet — the harness that proves kyma's five architectural invariants hold across every documented failure mode. From v1.0 forward, the claim *"kyma doesn't lose data"* is backed by a passing CI scenario for each way it could.

Deliverable: `scripts/gauntlet.sh` orchestrates six test families and runs in three CI tiers (PR / nightly / weekly). Every scenario maps to a specific invariant + failure mode from `docs/stability.md` and `docs/architecture.md`.

The six test families:

1. **Back-compat replay** — already shipped in F1; folded into the gauntlet by reference.
2. **Property + fuzz** — KQL parser fuzz, extent encoder/decoder roundtrip, catalog migration property tests.
3. **Deterministic simulation** — engine driven through scripted failure scenarios via mocks at existing trait boundaries.
4. **Chaos** — pg primary kill, MinIO 503 storm, network partitions, clock skew.
5. **Soak** — sustained-throughput ingest with periodic queries and health checks.
6. **Performance regression** — published thresholds for ingest/s and query p99 on a reference dataset.

### Non-goals for F2

- Feature coverage. The gauntlet is for *invariants*, not features. Existing `scripts/test-*.sh` keep feature coverage; F2 doesn't duplicate.
- Tokio scheduling-race testing (madsim/shuttle/turmoil). Kyma's invariants don't depend on scheduler-level determinism; converting the engine to a deterministic runtime is weeks of work disconnected from the v1.0 bar.
- Runner provisioning beyond GitHub Actions. The 24h soak runs as a manual / self-hosted RC qualification, not on hosted CI.
- Detecting code regressions outside the five architectural invariants. The gauntlet's job is "the contract still holds," not "all behavior unchanged."

---

## 2. Per-sub-spec definition of done

### F2.1 — Gauntlet orchestrator + CI tiers + perf baseline

Done when:

- `scripts/gauntlet.sh` exists; accepts `--tier=pr|nightly|weekly`; invokes the test families that tier owns; collects per-family pass/fail; exits non-zero if any fail.
- Three workflows wired up:
  - `.github/workflows/gauntlet-pr.yml` (runs on PR; ~5–10 min)
  - `.github/workflows/gauntlet-nightly.yml` (scheduled; ~1–2 h)
  - `.github/workflows/gauntlet-weekly.yml` (scheduled; 6 h cap on hosted runners).
- Perf-regression baseline: `scripts/perf-baseline.sh` runs a fixed workload against fresh kyma and emits `{ ingest_rps, query_p50_ms, query_p99_ms, ... }`. PR CI compares against a checked-in `scripts/fixtures/perf-baseline.json` and fails if a metric regresses past its tolerance band (e.g. `ingest_rps < 90% of baseline`, `query_p99 > 110% of baseline`).
- Reference dataset is a deterministic seed under `scripts/fixtures/perf-baseline/` (reuses or extends `scripts/seed-demo-data.sh`).
- Baseline numbers captured on `ubuntu-22.04` GHA hosted runner. Hardware notes documented alongside the baseline JSON.

### F2.2 — Property + fuzz

Done when:

- cargo-fuzz targets:
  - `crates/kyma-kql/fuzz/fuzz_targets/parse_kql_random.rs` — random bytes → parser must not panic; if it returns an error the error must be a structured `Result`, never `unreachable!()` or `panic!()`.
  - `crates/kyma-format-tlm/fuzz/fuzz_targets/extent_roundtrip.rs` — random valid extent bytes → decode → re-encode must equal input.
  - `crates/kyma-catalog/fuzz/fuzz_targets/migration_forward_only.rs` — randomly-generated migration sequences must satisfy the forward-only invariants from `docs/stability.md` §6.
- `proptest` tests in:
  - `crates/kyma-kql/` — arbitrary well-formed KQL parses → re-prints → re-parses to equivalent IR.
  - `crates/kyma-format-tlm/` — row → extent → row preserves all values (including dynamic, datetime, null).
  - `crates/kyma-catalog/` — a randomized series of valid migrations applied in order leaves a schema satisfying every forward-only rule.
- Seed corpora committed under `crates/*/fuzz/corpus/` (small, hand-picked tricky inputs).
- Tier coverage: PR runs proptest + 60s/fuzz target (smoke); nightly runs 10 min/fuzz target.
- Per-crate `rust-toolchain.toml` pinning nightly Rust for the fuzz subcrates only. Production crates stay on stable.

### F2.3 — Deterministic sim crate + invariant scenarios

Done when:

- New `crates/kyma-sim/` crate (test/dev-only, not shipped in release binaries). Provides:
  - Mock impls of `Catalog`, `ObjectStore`, `IngestRouter` traits with controllable failure injection (errors, delays, partial responses).
  - Per-scenario test harness that builds an engine wired to the mocks, runs a scripted sequence of operations, asserts invariants.
- **One scenario per architectural invariant** (the 5 from `docs/architecture.md`):
  1. Object store is source of truth → wipe local cache, restart, queries return same rows.
  2. Stateless compute → kill mid-ingest/query/commit/compaction → restart produces consistent state.
  3. Externalized catalog → pg failover mid-commit → idempotency keys survive, commit either fully lands or fully aborts.
  4. Pluggable format → mixed-format extents on the same table read correctly.
  5. Pluggable parser → KQL + SQL produce same logical plan for equivalent queries.
- **Plus one scenario per Axis-1 failure mode** from `docs/stability.md`:
  - ALTER TABLE mid-ingest never rewrites history; old extents read with null-fill.
  - S3 503 storms → bounded retries, no partial writes visible to readers.
  - Catalog ↔ object-store skew → GC reconciliation recovers.
  - Ingest exactly-once within the documented idempotency window.
- Each scenario is documented with: invariant name, failure mode being exercised, expected behavior, what would happen if the invariant didn't hold.

### F2.4 — Chaos extensions

Done when:

- `scripts/chaos/` directory replaces the single `chaos-test.sh`. Scenarios:
  - `pg-primary-kill.sh` — docker-compose kill postgres mid-ingest; verify engine reconnect + idempotency.
  - `minio-503-storm.sh` — inject 503s via toxiproxy in front of MinIO; verify bounded retries + no partial writes.
  - `network-partition.sh` — docker network disconnect between engine and pg/minio; verify graceful degradation.
  - `clock-skew.sh` — libfaketime sidecar shifts engine's clock; verify timestamp ordering invariants.
- Each scenario emits structured JSON `{ scenario, pass: bool, observations: [...] }` for `gauntlet.sh` to consume.
- Existing `chaos-test.sh` either deleted (replaced) or kept as a top-level wrapper calling the new scenarios.
- New tooling installed in CI: `toxiproxy-cli` (for 503 injection), `libfaketime` (for clock skew). Local-dev install instructions in `CONTRIBUTING.md`.

### F2.5 — Soak

Done when:

- `scripts/soak.sh --duration <Nh>` runs sustained ingest for N hours with periodic queries.
- Three health checks during soak:
  - **Liveness** — `/health` returns 200 every 30s.
  - **Memory** — RSS sampled every minute; fails if RSS grows monotonically (linear-fit slope > epsilon) past the first warm-up window. Epsilon tuned empirically during F2.5 implementation.
  - **Commit liveness** — no commit ID stuck in pending state for >30s.
- Soak in CI:
  - Nightly: 1h soak.
  - Weekly: 6h soak on GHA (hits the hosted-runner cap).
  - 24h soak: documented manual / self-hosted run, **not** on GHA. Runs as part of each RC qualification per the v1.0 master spec's M3 RC discipline.
- Soak failure artifact: tagged `.tar.gz` with last 1000 commit IDs, query-latency histogram, RSS-over-time CSV, engine logs (tail 50k lines), `/metrics` snapshots at start/middle/end.

---

## 3. Sub-spec decomposition

Five sub-specs. Each is sized for its own brainstorm → plan → execution cycle.

| Sub-spec | Scope | Independent of | Estimate |
|---|---|---|---|
| **F2.1** Orchestrator + CI tiers + perf baseline | Connective tissue; defines the tier contract every other sub-spec plugs into | n/a — comes first | 1 week |
| **F2.2** Property + fuzz | cargo-fuzz targets + proptest tests in 3 crates + nightly Rust pin for fuzz subcrates | F2.3, F2.4, F2.5 | 1–2 weeks |
| **F2.3** Deterministic sim + invariant scenarios | `crates/kyma-sim/` new dev crate + 9+ scenarios | F2.2, F2.4, F2.5 | **2–3 weeks (long pole)** |
| **F2.4** Chaos extensions | `scripts/chaos/` restructure + 4 new scenarios + toxiproxy/libfaketime | F2.2, F2.3, F2.5 | 1–2 weeks |
| **F2.5** Soak harness | `scripts/soak.sh` + 3 health checks + failure-artifact tarball | F2.2, F2.3, F2.4 | 1 week |

---

## 4. Sequencing

```
F2.1 (orchestrator) ──┬── F2.2 (property + fuzz)  ──┐
                      ├── F2.3 (deterministic sim) ──┤
                      ├── F2.4 (chaos extensions)  ──┼── all integrated into gauntlet.sh
                      └── F2.5 (soak)              ──┘
```

F2.1 ships first because it defines the tier contract and the invocation interface every other sub-spec plugs into. After F2.1, F2.2–F2.5 can be brainstormed/planned/executed in any order and even in parallel (different sub-agents on different sub-branches if desired).

Wall-clock budgets:

- Serial execution: ~6–9 weeks.
- F2.2–F2.5 in parallel: ~3–5 weeks.
- F2 fits inside the v1.0 M1 budget (6 weeks) only if at least some parallelism is used.

---

## 5. Decisions and risks

### Decisions taken (locked at master-spec time)

| Decision | Choice |
|---|---|
| F2 scope this round | **Full gauntlet now** (not skeleton + invariant-1 only). |
| F2 structure | **Master spec + 5 sub-specs**, same fractal pattern as v1.0 master. |
| Branching | F2 lands on `feature/f2-test-gauntlet` off main after F1 merged. |
| Deterministic sim approach | **Trait-injection-heavy** at existing `Catalog` / `ObjectStore` / `IngestRouter` boundaries. No runtime replacement (madsim/shuttle/turmoil) — out of scope for v1.0 invariants. |
| Fuzz toolchain | **cargo-fuzz** with nightly Rust pinned **per fuzz subcrate only** (production crates stay on stable). |
| Perf baseline strategy | **Pinned numbers** captured on `ubuntu-22.04` GHA hosted runner; re-baselined via documented procedure. |
| 24h soak in CI | **No.** Documented manual / self-hosted run per RC qualification. Weekly CI caps at 6h. |

### Risks to monitor during execution

- **F2.3's mock surface may require trait changes.** Current `Catalog` / `ObjectStore` / `IngestRouter` traits were designed for production impls, not failure injection. F2.3's brainstorm includes a trait-surface audit as its first task. Mitigation: if hooks are missing, extend the traits in F2.3 with `#[cfg(test)]` injection middleware rather than wholesale rewrites.
- **Actions billing still blocks CI** (same constraint that hit F1). F2.1's workflow validation requires Actions to be unblocked. F2.2–F2.5 can build + validate locally; their CI tier wiring only runs once billing is resolved.
- **`RSS-grows-monotonically` check in F2.5 is heuristic.** Rust + Tokio + jemalloc/system-malloc don't return RSS smoothly. F2.5 implementation will calibrate the slope threshold and warm-up window on a clean 1h baseline run before turning the check from advisory to gating.
- **toxiproxy / libfaketime adoption.** Both add to CI runner setup time and to local dev environment. `gauntlet.sh` must print a structured "missing tool" error rather than silently skipping a scenario.
- **Sim crate + format-v1 (P0) interaction.** When P0 (format v1) lands, the extent format changes. F2.3's "pluggable format" scenario must accommodate both format-v0 (current) and format-v1 (post-P0) extents. F2.3's brainstorm explicitly designs the mock to be format-version-agnostic.
- **Perf baseline re-capture cost.** When GHA changes hosted runner hardware (which they do periodically), the baseline numbers shift. The re-baseline procedure documented in F2.1 must be easy to follow; ideally a one-command operation.

---

## Appendix — Sub-spec stubs

Each stub is the seed for a per-sub-spec brainstorming/writing-plans cycle. They are not implementation plans — they list the work the sub-spec must scope.

### F2.1 — Orchestrator + CI tiers + perf baseline

- Design the tier contract: what does `gauntlet.sh --tier=pr` invoke vs `--tier=nightly` vs `--tier=weekly`? Output shape, exit codes, log structure.
- Write `scripts/gauntlet.sh` with placeholder dispatch to the future F2.2–F2.5 commands.
- Three workflow files (`.github/workflows/gauntlet-{pr,nightly,weekly}.yml`).
- `scripts/perf-baseline.sh` — runs a fixed workload, emits JSON metrics.
- `scripts/perf-check.sh` — compares current run against `scripts/fixtures/perf-baseline.json`, fails on regression.
- Capture initial baseline numbers on `ubuntu-22.04` GHA runner; commit `scripts/fixtures/perf-baseline.json`.
- Document the re-baseline procedure.

### F2.2 — Property + fuzz

- Audit the KQL parser, extent format, and catalog migration surfaces for fuzz attack points.
- Add cargo-fuzz subcrates under `crates/{kyma-kql,kyma-format-tlm,kyma-catalog}/fuzz/`.
- Pin nightly Rust per fuzz subcrate via `rust-toolchain.toml`.
- Write fuzz targets per Section 2 F2.2.
- Build seed corpora.
- Add proptest tests per Section 2 F2.2.
- Wire fuzz targets into `gauntlet.sh` tier dispatch (PR: 60s/target; nightly: 10 min/target).

### F2.3 — Deterministic sim + invariant scenarios

- **First task: trait-surface audit.** Read `Catalog`, `ObjectStore`, `IngestRouter` trait definitions. List the failure-injection hooks each scenario needs. Decide: extend traits, wrap with middleware, or both?
- Create `crates/kyma-sim/` dev crate.
- Build mock impls of the 3 traits with injection control.
- Write the 5 invariant scenarios.
- Write the 4 Axis-1 failure-mode scenarios.
- Document each scenario (invariant name, failure mode, expected behavior, what failure would mean).
- Wire scenarios into `gauntlet.sh` (PR: subset; nightly: full).

### F2.4 — Chaos extensions

- Restructure `scripts/chaos/` directory.
- Each chaos scenario as a separate script emitting structured JSON.
- Install toxiproxy + libfaketime in CI workflow + document local install.
- Write the 4 scenarios per Section 2 F2.4.
- Either delete the old `scripts/chaos-test.sh` or convert to a top-level wrapper.
- Wire into `gauntlet.sh` nightly tier.

### F2.5 — Soak

- `scripts/soak.sh` with configurable duration.
- Liveness check (loop, 30s interval).
- Memory check: RSS sampler + linear-fit-slope evaluator + tunable threshold.
- Commit-liveness check: poll catalog for stuck commit IDs.
- Failure-artifact bundler.
- Calibrate RSS slope threshold on a clean 1h run before turning the check from advisory to gating.
- Document the 24h manual-run procedure for RC qualification.
- Wire into `gauntlet.sh` nightly (1h) and weekly (6h) tiers.
