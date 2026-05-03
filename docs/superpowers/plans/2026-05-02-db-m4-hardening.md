# DB Integration M4 — Hardening & Docs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Prerequisite:** [DB M3 Mongo](2026-05-02-db-m3-mongo.md) is complete and committed.

**Goal:** Take the three-engine integration over the line for v1. Web UI for connector lifecycle (create/edit/pause/resume/status without curl), `/v1/connectors/:id/events` polish, agent-tool integration over external sources via the existing `schema_embeddings` mechanism, formal docs site pages for the Connectors marquee section (assuming Docs D2 has shipped — if not, ship `<PushdownMatrix>` / `<SchemaMappingTable>` data files first), and a sweep of remaining loose ends from M1/M2/M3 (LSN advance accuracy, full keepalive replies, primary-PG-version coverage on benchmarks).

**Architecture:** The web UI lives in `web/` (the existing kyma web workspace, per the existing `2026-04-20-kyma-web-ui-foundation-and-workspace.md` plan). M4 adds a Connectors page set: list, detail (status doc), create form (mode-aware), edit, events trail. Agent integration extends `kyma-server::agent::tools` to teach the existing `describe_table`/`run_sql`/`list_databases` tools about external sources via the catalog provider abstractions added in M0/M1 — the agent already gets a unified view because everything is in the DataFusion `SessionContext`, but schema RAG over external schemas needs the embedding step. Docs pages render `<PushdownMatrix>` / `<SchemaMappingTable>` from the JSON exported by `kyma-cli docs-export` (Docs D2 milestone).

**Tech Stack:** TypeScript 5, Vue 3.5 (or React per the kyma web foundation plan — verify which), kyma-server admin endpoints (already exist from M0), VitePress (docs site), pgvector (existing embedding store). Spec: [`docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md`](../specs/2026-05-02-multi-source-database-integration-design.md).

---

## File Structure

**New files:**

- `web/src/views/connectors/ConnectorList.vue` (or React equivalent — verify against `web/src-tauri` and `web/src/`)
- `web/src/views/connectors/ConnectorDetail.vue`
- `web/src/views/connectors/ConnectorCreateForm.vue`
- `web/src/views/connectors/ConnectorEvents.vue`
- `web/src/api/connectors.ts` — typed client over `/v1/connectors/*` endpoints.
- `crates/kyma-server/src/agent/tools/external.rs` — agent tool extensions: schema discovery over registered external sources.
- `crates/kyma-server/src/agent/embedding.rs` — extension point that emits `InferredSchema` rows into `schema_embeddings` whenever a connector is created or its introspection cache is refreshed.
- `docs/site/connectors/multi-source-data.md` — the marquee page (depends on Docs D1).
- `docs/site/connectors/postgres.md`, `docs/site/connectors/mysql.md`, `docs/site/connectors/mongo.md` — engine pages with `<PushdownMatrix>` + `<SchemaMappingTable>` (depend on Docs D2).
- `docs/site/concepts/multi-source-data.md` — concept page covering the federation/sync model, `live(...)`, exactly-once.

**Modified files:**

- `crates/kyma-connectors/src/external/postgres/replication.rs` — replace M1 `TODO_advance` with proper LSN tracking via `Commit` events; full standby-status keepalive replies.
- `crates/kyma-connectors/src/external/mysql/replication.rs` — full GTID advance accuracy.
- `crates/kyma-connectors/src/external/mongo/change_stream.rs` — full `resumeToken`-based resume (already correct; verify edge cases).
- `crates/kyma-server/src/connectors/extensions.rs` — populate the `federation` block of the status doc with real pool stats + p50/p99 latency from metrics. Populate the `events_per_sec` and `pool_in_use` columns of `kyma_connector_health`.
- `crates/kyma-bin/src/main.rs` — register external-source schema embeddings on connector lifecycle.

---

## Task 1: Replication LSN/GTID accuracy sweep

**Files:**
- Modify: `crates/kyma-connectors/src/external/postgres/replication.rs`
- Modify: `crates/kyma-connectors/src/external/mysql/replication.rs`

- [ ] **Step 1: Postgres** — replace the `"TODO_advance"` placeholder. The `pgoutput` Commit message carries the commit LSN; track it across the message stream and emit on each event:
  - Maintain a running `last_commit_lsn`.
  - When a `Begin/.../Commit` group commits, emit one `CdcEvent` per row in the group, all with the same `Checkpoint{lsn: last_commit_lsn, slot: ...}`.
  - Reply to PrimaryKeepalive with a CopyData status update carrying `last_commit_lsn` (so Postgres can free WAL).

- [ ] **Step 2: MySQL** — similar pass. Track GTID via the `GTID` event preceding each transaction; emit it as `Checkpoint{gtid: ...}` on every event in that transaction.

- [ ] **Step 3: Tests**

```rust
#[tokio::test]
async fn cdc_events_carry_monotonically_advancing_checkpoints() {
    // Insert 1000 rows in batches, observe stream, assert checkpoints monotonically increase.
}

#[tokio::test]
async fn keepalive_reply_does_not_block_event_stream() {
    // Idle for 30s with no events, then insert; confirm the connection stays alive.
}
```

- [ ] **Step 4: Re-run chaos suite** (Tasks M1#13 and M2#12) to confirm exactly-once still holds with accurate LSNs.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/replication.rs crates/kyma-connectors/src/external/mysql/replication.rs
git commit -m "fix(db): accurate LSN/GTID advance + keepalive replies"
```

---

## Task 2: Status endpoint completeness

**Files:**
- Modify: `crates/kyma-server/src/connectors/extensions.rs`
- Modify: `crates/kyma-federation/src/scan_exec.rs`

- [ ] **Step 1: Federation pool stats.** `kyma-federation::scan_exec` records into a `metrics::Counter` per connector each time it acquires/releases a pool conn. The status handler reads the latest values via the `metrics` registry and populates `federation.pool_in_use` / `federation.pool_max`.

- [ ] **Step 2: Federation latency** — record `scan_duration_ms` per query into a histogram (`metrics::Histogram`), then surface p50/p99 in the status doc.

- [ ] **Step 3: `kyma_connector_health` view backfill** — alter the view to JOIN against the same metrics-derived health table (or expose the metrics via a SQL function so the view can read live values). Simpler approach: a small in-memory snapshot table refreshed every N seconds, exposed as `kyma_connector_health_metrics` and joined into the existing view.

- [ ] **Step 4: Test**

```rust
#[tokio::test]
async fn status_endpoint_reflects_actual_pool_use() {
    // Acquire 3 conns from the pool, hold them, hit /status, assert pool_in_use >= 3.
}

#[tokio::test]
async fn kyma_connector_health_table_returns_live_lag() {
    // Run a sync connector for 30s; query `SELECT * FROM kyma_connector_health` via /v1/query.
    // Assert lag_seconds is bounded.
}
```

- [ ] **Step 5: Commit**

---

## Task 3: Web UI — connector list

**Files:**
- Create: `web/src/views/connectors/ConnectorList.vue`
- Create: `web/src/api/connectors.ts`

- [ ] **Step 1: Read the existing web stack.** `web/` has its own `package.json`. Confirm Vue vs React; this plan assumes Vue. If React, translate the components.

- [ ] **Step 2: Typed API client** for `/v1/connectors`, `/v1/connectors/:id`, `/v1/connectors/:id/status`, `/v1/connectors/:id/events`, `/v1/connectors/:id/test-connection`, pause/resume.

- [ ] **Step 3: List page**: table of connectors with columns (name, type, mode, phase, lag, last_error, actions). Polling every 5s for status freshness.

- [ ] **Step 4: Smoke test** in browser against a running kyma + Postgres connector.

- [ ] **Step 5: Commit**

---

## Task 4: Web UI — connector detail + events

**Files:**
- Create: `web/src/views/connectors/ConnectorDetail.vue`
- Create: `web/src/views/connectors/ConnectorEvents.vue`

- [ ] **Step 1: Detail page** renders the structured status JSON: source health card, federation card (pool, latency, errors), sync card (per source-table phases, lag, schema drift list, errors).

- [ ] **Step 2: Events tab** — paginated list from `/v1/connectors/:id/events`. Each row shows `kind`, `occurred_at`, payload preview.

- [ ] **Step 3: Pause / resume buttons** call `/pause?scope=...` and `/resume`. Test connection button calls `/test-connection`.

- [ ] **Step 4: Commit**

---

## Task 5: Web UI — connector create form (mode-aware)

**Files:**
- Create: `web/src/views/connectors/ConnectorCreateForm.vue`

- [ ] **Step 1: Form fields** for `type` (radio: postgres/mysql/mongo/prometheus), `mode` (radio: sync/federation/both — disabled for prometheus). When type is one of the DB engines, show:
  - Connection block: URL, secret_ref dropdown (queries an `/v1/secrets/refs` endpoint that lists known refs), TLS mode, pool_size.
  - Scope block: schema include list, table exclude list, flatten_depth (Mongo only), decimal128_mode, geometry_mode (Postgres only).
  - Sync block (when mode includes sync): table list, schedule_ms.

- [ ] **Step 2: Test-before-save** — calls `/v1/connectors/:id/test-connection` (or a pre-create variant; the spec calls out this endpoint shape) and shows the result inline before letting the user submit.

- [ ] **Step 3: Submit → POST /v1/connectors** with a graceful error UI when secret refs don't resolve.

- [ ] **Step 4: Commit**

---

## Task 6: Agent integration — schema embeddings for external sources

**Files:**
- Modify: `crates/kyma-server/src/agent/embedding.rs` (or wherever schema embeddings already live)
- Modify: `crates/kyma-server/src/agent/tools/external.rs` (new)

- [ ] **Step 1: When a connector is created or its introspection cache is refreshed, emit `InferredSchema` rows into the existing `schema_embeddings` table** (already used for kyma-native tables). The schema-text-to-vector pipeline already exists per the agent-and-vectors spec; we just feed it new rows.

- [ ] **Step 2: Update agent tools** so `list_databases` lists kyma + each registered external source; `describe_table` resolves three-part names; `run_sql` queries against the unified `SessionContext`.

- [ ] **Step 3: Eval test** — give the agent the prompt "what's our top customer's recent error rate?" against a database with a federated `pg_prod.public.customers` table and a kyma-native `otel_logs` table. Assert the agent emits a SQL query that JOINs both. Use a small fixed eval suite checked in.

- [ ] **Step 4: Commit**

---

## Task 7: Docs pages — multi-source data marquee + per-engine pages

**Files:**
- Create: `docs/site/concepts/multi-source-data.md`
- Create: `docs/site/connectors/multi-source-data.md`
- Create: `docs/site/connectors/postgres.md`
- Create: `docs/site/connectors/mysql.md`
- Create: `docs/site/connectors/mongo.md`

(Depends on Docs D1 for the site shell and D2 for the `<PushdownMatrix>` / `<SchemaMappingTable>` data + components.)

- [ ] **Step 1: Concept page** — covers the federation+sync model, why both, exactly-once, `live(...)` UX. Copy the prose from spec §1, §3, §5, §9.4 — adjusted for end-user voice.

- [ ] **Step 2: Per-engine pages** — three pages following one template:
  1. Quickstart (POST /v1/connectors body, expected status response).
  2. `<SchemaMappingTable engine="postgres" />` (rendered from `schema-mappings.json`).
  3. `<PushdownMatrix engine="postgres" />` (rendered from `pushdown-capabilities.json`).
  4. CDC mechanism (replication slots / binlog / change streams).
  5. Known limitations + roadmap (link to the spec's deferred-to-v1.5 list).
  6. Troubleshooting (status interpretation, common operator-actionable errors).

- [ ] **Step 3: One worked example per engine** — a `kql-runnable` snippet showing a cross-source join. (Validated by Docs D3 doctests.)

- [ ] **Step 4: Commit**

---

## Task 8: Docs — `live(...)`, mode-resolution rules, observability page

**Files:**
- Modify: `docs/site/connectors/multi-source-data.md`
- Create: `docs/site/concepts/observability-for-connectors.md`

- [ ] **Step 1: `live(...)` reference** — spec §5.4 / §9.4 prose, with KQL + SQL examples. Calls out: "synced wins by default; `live(table)` opts into federation."

- [ ] **Step 2: Observability page** — covers `pushdown_summary` schema (with example), the `kyma_connector_health` table schema, the `/v1/connectors/:id/status` and `/events` endpoints. Show how to detect "what got pushed down" via a pasted summary.

- [ ] **Step 3: Commit**

---

## Task 9: Final acceptance — full M0–M4 sweep

- [ ] `cargo build --workspace --all-targets --features postgres,mysql,mongo,federation` clean.
- [ ] `cargo test --workspace --all-targets` clean.
- [ ] All chaos suites pass nightly.
- [ ] Property tests (100k cases nightly) clean for all three engines.
- [ ] Performance benches within budget on at least 3 consecutive runs.
- [ ] Web UI usable end-to-end: a non-developer can create / pause / resume / inspect / delete a connector via browser only.
- [ ] Agent eval suite passes cross-source questions.
- [ ] Docs pages render the generated tables correctly; doctests pass on every runnable code block.
- [ ] `pushdown_summary` is present and non-empty on every federated response in production.
- [ ] `kyma_connector_health` returns sensible values for every active connector.
- [ ] Tag `db-m4-hardening`. Optionally `db-v1-ga`.

---

## Cross-cutting follow-ups baked into the spec (out of v1)

For reference; **not** in scope for M4. From spec §13.5:

- v1.5: JSONB / MySQL-JSON drill-in, typed Arrow `List` for arrays, connector alerts/SLOs in config, first-class bytes columns, concurrent-DDL exclusion lock on synced tables.
- v2: more engines (SQLite / SQL Server / Oracle / DynamoDB / ClickHouse / Snowflake-as-source / BigQuery-as-source), read-time consistency fences, multi-region federation.
- Post-v1: window-function pushdown, auto-recreate dropped replication slot with operator sign-off.

Each is a separate spec → plan cycle, not a follow-on milestone of this plan.
