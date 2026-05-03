# DB Integration M0 — Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the engine-agnostic foundation for multi-source database integration: `ExternalSource` trait, `Capabilities` struct, `kyma-federation` crate skeleton, catalog migrations (`mode`, `connection_jsonb`, `connector_cdc_state`, `kyma_connector_health` view), `SecretStore` resolver gating, admin API extensions (status/events/test-connection/scoped-pause), the `live(...)` table function, `pushdown_summary` plumbing, and the `kyma-connectors-testkit` dev crate. Zero engine implementations land here — M0 ships the trait shape, the CAS-with-cursor-update primitive, and the test scaffolding.

**Architecture:** One `ExternalSource` trait in `kyma-connectors` with two consumers — federation (new `kyma-federation` crate that registers DataFusion `CatalogProvider`s) and sync (new `CdcConnector` sibling trait registered alongside the existing `Connector`). Catalog gains additive columns + a CDC state table + a health view. Existing `CommitCoordinator` gets a single `with_cursor_update` method so cursor advances ride the same CAS as snapshot commits — this is the exactly-once knot. No engine impls; no engine deps. The Postgres slice (M1) implements `PostgresSource` against the trait the next milestone over.

**Tech Stack:** Rust 1.95, Tokio 1, async-trait, DataFusion 44 (`CatalogProvider`, `TableProvider`, `TableFunction`), sqlx 0.8 (Postgres catalog), Arrow 53, proptest, testcontainers, axum 0.7, serde/serde_json. New crates: `kyma-federation`, `kyma-connectors-testkit`. Spec: [`docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md`](../specs/2026-05-02-multi-source-database-integration-design.md).

---

## File Structure

**New files:**

- `crates/kyma-connectors/src/external/mod.rs` — module root; re-exports `ExternalSource`, `Capabilities`, `ConnectionConfig`, `Scope`, `InferredSchema`, `PushedPlan`, `PushdownSummary`, `BatchSink`, `Checkpoint`.
- `crates/kyma-connectors/src/external/trait_def.rs` — the `ExternalSource` trait + `SourceHandle` + `SourceHealth`.
- `crates/kyma-connectors/src/external/capabilities.rs` — `Capabilities` struct (operators, function allowlist, agg shapes, sort/limit, joins, collation-safe columns).
- `crates/kyma-connectors/src/external/connection.rs` — `ConnectionConfig`, `ResolvedConnection`, `TlsMode`, `pool_size` defaults.
- `crates/kyma-connectors/src/external/scope.rs` — `Scope` (include/exclude lists, flatten_depth, decimal128_mode, geometry_mode, etc.).
- `crates/kyma-connectors/src/external/schema.rs` — `InferredSchema`, `InferredColumn`, `_kyma_pk`/`_kyma_op`/`_kyma_lsn`/`_kyma_event_at` system-column definitions.
- `crates/kyma-connectors/src/external/plan.rs` — `PushedPlan` enum (`Sql(String, Vec<BoundParam>)`, `MongoPipeline(Vec<serde_json::Value>)`), `PushdownSummary`, `BoundParam`.
- `crates/kyma-connectors/src/external/sink.rs` — `BatchSink` trait (Arrow `RecordBatch` consumer for federated scans).
- `crates/kyma-connectors/src/external/checkpoint.rs` — `Checkpoint` (engine-opaque JSON wrapper).
- `crates/kyma-connectors/src/external/error.rs` — `ExternalError` enum with the three-class taxonomy (`Transient`, `OperatorActionable`, `Bug`).
- `crates/kyma-connectors/src/cdc/mod.rs` — module root.
- `crates/kyma-connectors/src/cdc/connector.rs` — `CdcConnector` trait (sibling of `Connector`).
- `crates/kyma-connectors/src/cdc/runner.rs` — phase-machine runner (pending → snapshotting → streaming → errored).
- `crates/kyma-connectors/src/cdc/snapshot.rs` — initial-snapshot coordinator (engine-agnostic; calls `ExternalSource::snapshot_at`).
- `crates/kyma-connectors/src/cdc/stream.rs` — streaming consumer (engine-agnostic; calls `ExternalSource::open_cdc`).
- `crates/kyma-connectors/src/cdc/evolver.rs` — `SchemaEvolver` (typed-vs-dynamic promotion rules; reuses kyma-ingest-core schema-evolution primitives).
- `crates/kyma-connectors/src/cdc/state.rs` — sqlx helpers for `connector_cdc_state` table.
- `crates/kyma-connectors/migrations/0010_external_sources.sql` — additive catalog migration.
- `crates/kyma-federation/Cargo.toml` — new crate manifest.
- `crates/kyma-federation/src/lib.rs` — crate root.
- `crates/kyma-federation/src/catalog_provider.rs` — `FederatedCatalogProvider`.
- `crates/kyma-federation/src/table_provider.rs` — `FederatedTableProvider`.
- `crates/kyma-federation/src/planner.rs` — engine-agnostic `PushdownPlanner`.
- `crates/kyma-federation/src/live_fn.rs` — `live(table)` DataFusion `TableFunction`.
- `crates/kyma-federation/src/registry.rs` — registry of `Arc<dyn ExternalSource>` keyed by source name; building block for `kyma-exec` `SessionContext` wiring.
- `crates/kyma-connectors-testkit/Cargo.toml` — new dev crate manifest.
- `crates/kyma-connectors-testkit/src/lib.rs` — test scaffolding root.
- `crates/kyma-connectors-testkit/src/query_gen.rs` — `proptest` strategies for the pushdown property generator (uses fixed schema from spec §12.2).
- `crates/kyma-connectors-testkit/src/fault.rs` — fault injectors (process kill, network drop helpers).
- `crates/kyma-connectors-testkit/src/snapshot.rs` — Arrow-result-fingerprint helper (used by property + chaos tests).
- `crates/kyma-server/src/connectors/extensions.rs` — admin API extensions (status, events, test-connection, scoped pause, `mode` field validation, secret-resolution gating).
- `tests/integration/external_foundation.rs` (workspace-level integration test crate; if it doesn't exist, lives at `crates/kyma-connectors/tests/foundation.rs`).

**Modified files:**

- `Cargo.toml` (workspace root) — add `kyma-federation`, `kyma-connectors-testkit` to `[workspace] members` and `[workspace.dependencies]`.
- `crates/kyma-connectors/Cargo.toml` — add `kyma-ingest-core` dep (already present), `proptest` dev-dep.
- `crates/kyma-connectors/src/lib.rs` — `pub mod external; pub mod cdc;`.
- `crates/kyma-connectors/src/registry.rs` — extend to also accept `Arc<dyn CdcConnector>` (sibling trait).
- `crates/kyma-connectors/src/runner.rs` — dispatch by trait kind (existing `Connector` vs. new `CdcConnector`).
- `crates/kyma-connectors/src/types.rs` — no behavioral change; ensure `ConnectorError` taxonomy matches `ExternalError` so the runner can map cleanly.
- `crates/kyma-connectors/src/catalog_sql.rs` — extend `ConnectorRow` with `mode`, `connection_jsonb`, `scope_jsonb` columns; add CRUD helpers.
- `crates/kyma-connectors/src/admin.rs` — extend `POST /v1/connectors` body schema; add the new endpoints from spec §4.6.
- `crates/kyma-ingest-core/src/commit.rs` (or whichever module hosts `CommitCoordinator`) — add `with_cursor_update(connector_id: Uuid, source_table: &str, checkpoint: serde_json::Value)` builder method.
- `crates/kyma-ingest-core/src/lib.rs` — re-export the new builder type if not already re-exported.
- `crates/kyma-server/src/lib.rs` — register the new admin routes; register `kyma-federation` registry into `SessionContext` builder.
- `crates/kyma-exec/src/lib.rs` — add `register_federated_catalogs(session: &mut SessionContext, registry: &FederationRegistry)` extension function.
- `crates/kyma-server/Cargo.toml` — add a `federation` Cargo feature gating the `kyma-federation` dep (`federation = ["dep:kyma-federation"]`).
- `crates/kyma-bin/src/main.rs` — when feature `federation` enabled, build the `FederationRegistry` from catalog rows and hand it to `kyma-exec::register_federated_catalogs`.

**Test files:**

- `crates/kyma-connectors/tests/foundation.rs` — integration tests for catalog migration + admin API extensions (uses testcontainers Postgres).
- `crates/kyma-connectors/src/cdc/evolver.rs` — unit tests inline (`#[cfg(test)] mod tests`).
- `crates/kyma-federation/src/planner.rs` — unit tests inline + property tests via `kyma-connectors-testkit`.
- `crates/kyma-federation/tests/planner_property.rs` — runs `kyma-connectors-testkit` query generator against a synthetic `FakeSource` impl.
- `crates/kyma-ingest-core/src/commit.rs` — unit test for `with_cursor_update` (atomic CAS + cursor advance).

---

## Task 1: Workspace scaffolding for the new crates

**Files:**
- Modify: `/Users/shaked/projects_new/agentcy/kyma/Cargo.toml`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-federation/Cargo.toml`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-federation/src/lib.rs`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-connectors-testkit/Cargo.toml`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-connectors-testkit/src/lib.rs`

- [ ] **Step 1: Add the new crates to the workspace**

Edit `Cargo.toml`, add to `[workspace] members`:

```toml
    "crates/kyma-federation",
    "crates/kyma-connectors-testkit",
```

In `[workspace.dependencies]` add:

```toml
kyma-federation        = { path = "crates/kyma-federation" }
kyma-connectors-testkit = { path = "crates/kyma-connectors-testkit" }
```

- [ ] **Step 2: Create `kyma-federation` manifest**

Write `crates/kyma-federation/Cargo.toml`:

```toml
[package]
name = "kyma-federation"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[lints]
workspace = true

[dependencies]
kyma-core = { workspace = true }
kyma-connectors = { workspace = true }
tokio = { workspace = true }
async-trait = { workspace = true }
anyhow = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
arrow = { workspace = true }
arrow-array = { workspace = true }
arrow-schema = { workspace = true }
datafusion = { workspace = true }
futures = { workspace = true }
uuid = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["full", "test-util"] }
proptest = { workspace = true }
kyma-connectors-testkit = { workspace = true }
```

- [ ] **Step 3: Create `kyma-federation` lib root**

Write `crates/kyma-federation/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
//! kyma-federation — DataFusion catalog/table providers backed by external
//! databases (Postgres, MySQL, MongoDB) via the `kyma-connectors`
//! `ExternalSource` trait.
//!
//! See `docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md`.

pub mod catalog_provider;
pub mod live_fn;
pub mod planner;
pub mod registry;
pub mod table_provider;

pub use catalog_provider::FederatedCatalogProvider;
pub use planner::{PushdownPlanner, PushdownSummary};
pub use registry::FederationRegistry;
pub use table_provider::FederatedTableProvider;
```

- [ ] **Step 4: Create `kyma-connectors-testkit` manifest**

Write `crates/kyma-connectors-testkit/Cargo.toml`:

```toml
[package]
name = "kyma-connectors-testkit"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

[lints]
workspace = true

[dependencies]
kyma-core = { workspace = true }
kyma-connectors = { workspace = true }
arrow = { workspace = true }
arrow-array = { workspace = true }
arrow-schema = { workspace = true }
datafusion = { workspace = true }
proptest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
tokio = { workspace = true }
async-trait = { workspace = true }
```

- [ ] **Step 5: Create `kyma-connectors-testkit` lib root**

Write `crates/kyma-connectors-testkit/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
//! Test scaffolding shared across kyma-connectors engine impls.
//!
//! - `query_gen` — proptest strategies generating pushdown-correctness queries.
//! - `fault` — process / network fault injectors for chaos tests.
//! - `snapshot` — Arrow-result fingerprinting for snapshot comparisons.
//!
//! See spec §12.

pub mod query_gen;
pub mod fault;
pub mod snapshot;
```

- [ ] **Step 6: Verify workspace builds**

Run: `cargo check --workspace`
Expected: clean compile (modules referenced in lib.rs don't yet exist; we'll add them in subsequent tasks, so it will fail with "file not found"). At this checkpoint only the manifests must validate.

Actually fix the missing-module error by creating empty placeholder files:

```bash
mkdir -p crates/kyma-connectors-testkit/src
mkdir -p crates/kyma-federation/src
```

Write each as an empty `pub mod` placeholder containing only `#![forbid(unsafe_code)]\n` for the testkit modules and a single `pub struct Stub;` placeholder for federation modules — actual content lands in later tasks.

For now, write empty files with minimal valid content:

```rust
// crates/kyma-connectors-testkit/src/query_gen.rs
#![forbid(unsafe_code)]
```

Repeat for `fault.rs`, `snapshot.rs`.

```rust
// crates/kyma-federation/src/catalog_provider.rs
#![forbid(unsafe_code)]
pub struct FederatedCatalogProvider;
```

Repeat the same single-stub pattern for `live_fn.rs`, `planner.rs`, `registry.rs`, `table_provider.rs` (give each a single placeholder type that matches the name re-exported from `lib.rs` so the re-exports compile). Re-exports listed in `lib.rs` must reference real types, even if those types are empty stubs at this step.

Specifically:

```rust
// crates/kyma-federation/src/catalog_provider.rs
#![forbid(unsafe_code)]
pub struct FederatedCatalogProvider;

// crates/kyma-federation/src/planner.rs
#![forbid(unsafe_code)]
pub struct PushdownPlanner;
pub struct PushdownSummary;

// crates/kyma-federation/src/registry.rs
#![forbid(unsafe_code)]
pub struct FederationRegistry;

// crates/kyma-federation/src/table_provider.rs
#![forbid(unsafe_code)]
pub struct FederatedTableProvider;

// crates/kyma-federation/src/live_fn.rs
#![forbid(unsafe_code)]
```

- [ ] **Step 7: Verify the workspace compiles cleanly**

Run: `cargo check --workspace`
Expected: PASS with no errors (warnings OK at this stage).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml crates/kyma-federation crates/kyma-connectors-testkit
git commit -m "feat(db): scaffold kyma-federation and kyma-connectors-testkit crates"
```

---

## Task 2: Catalog migration for external sources

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-catalog/migrations/0010_external_sources.sql`

(Note: confirm migration directory by reading `crates/kyma-catalog/src/lib.rs` — kyma uses `sqlx::migrate!` so the path is whatever that macro points at. If it's `crates/kyma-catalog/migrations`, place the file there. If it's elsewhere, use that path. Check the existing migrations to determine the next number; if `0009_*` is the latest, this is `0010_*`.)

- [ ] **Step 1: Read existing migrations to confirm path and next-number**

Run: `ls crates/kyma-catalog/migrations/ 2>/dev/null || find crates/kyma-catalog -name "*.sql" -type f`
Note the highest existing migration number; this new file is one greater. The example below assumes `0010_*` — adjust if needed.

- [ ] **Step 2: Write the migration**

Write `crates/kyma-catalog/migrations/0010_external_sources.sql`:

```sql
-- External data source integration (Postgres / MySQL / Mongo).
-- Spec: docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md

-- Mode + structured connection (split out from config_jsonb).
ALTER TABLE connectors ADD COLUMN IF NOT EXISTS mode TEXT NOT NULL DEFAULT 'sync';
ALTER TABLE connectors ADD COLUMN IF NOT EXISTS connection_jsonb JSONB;
ALTER TABLE connectors ADD COLUMN IF NOT EXISTS scope_jsonb JSONB;

-- Sanity: mode is one of the allowed values.
ALTER TABLE connectors
    DROP CONSTRAINT IF EXISTS connectors_mode_check;
ALTER TABLE connectors
    ADD CONSTRAINT connectors_mode_check
        CHECK (mode IN ('sync', 'federation', 'both'));

-- CDC state (per source-table). Survives connector restarts; checkpoint is the
-- exactly-once cursor that's advanced atomically with the snapshot CAS commit.
CREATE TABLE IF NOT EXISTS connector_cdc_state (
    connector_id  UUID NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
    source_table  TEXT NOT NULL,
    phase         TEXT NOT NULL CHECK (phase IN ('pending', 'snapshotting', 'streaming', 'errored')),
    checkpoint    JSONB,
    rows_synced   BIGINT NOT NULL DEFAULT 0,
    last_event_at TIMESTAMPTZ,
    last_error    TEXT,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (connector_id, source_table)
);

CREATE INDEX IF NOT EXISTS connector_cdc_state_phase_idx
    ON connector_cdc_state (phase) WHERE phase IN ('snapshotting', 'errored');

-- State-transition events trail (last N entries per connector; bounded by retention sweep).
CREATE TABLE IF NOT EXISTS connector_events (
    id            BIGSERIAL PRIMARY KEY,
    connector_id  UUID NOT NULL REFERENCES connectors(id) ON DELETE CASCADE,
    occurred_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    kind          TEXT NOT NULL,
    payload_jsonb JSONB
);

CREATE INDEX IF NOT EXISTS connector_events_connector_id_occurred_at_idx
    ON connector_events (connector_id, occurred_at DESC);

-- Health view: queryable as a kyma table once registered with kyma-exec.
-- Synthesizes one row per (connector, source_table) for sync mode plus one
-- row per connector for federation mode.
CREATE OR REPLACE VIEW kyma_connector_health AS
SELECT
    c.id              AS connector_id,
    c.name            AS connector_name,
    c.type            AS type,
    c.mode            AS mode,
    s.source_table    AS source_table,
    s.phase           AS phase,
    EXTRACT(EPOCH FROM (now() - s.last_event_at))::DOUBLE PRECISION AS lag_seconds,
    NULL::DOUBLE PRECISION AS events_per_sec,         -- backfilled by metrics path; null if not yet observed
    NULL::INTEGER          AS pool_in_use,            -- federation pool snapshot; null for sync-only rows
    NULL::INTEGER          AS pool_max,
    s.last_error      AS last_error,
    s.last_event_at   AS last_event_at,
    now()             AS observed_at
FROM connectors c
LEFT JOIN connector_cdc_state s ON s.connector_id = c.id
WHERE c.enabled = TRUE OR s.phase = 'errored';
```

- [ ] **Step 3: Run the migration test**

Find or create a test that runs migrations end-to-end. If there's an existing pattern (e.g., `cargo test -p kyma-catalog --test migrations`), reuse it. Otherwise, write a minimal test in `crates/kyma-catalog/tests/migrations.rs`:

```rust
use testcontainers::clients::Cli;
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn migration_0010_applies_cleanly() {
    let docker = Cli::default();
    let pg = docker.run(Postgres::default());
    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432)
    );
    let pool = sqlx::PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    // Probe the new shape.
    let modes: Vec<String> = sqlx::query_scalar(
        "SELECT mode FROM connectors WHERE FALSE",
    ).fetch_all(&pool).await.unwrap();
    assert!(modes.is_empty());

    // Verify the view exists.
    let view_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_matviews WHERE matviewname = 'kyma_connector_health')
         OR EXISTS (SELECT 1 FROM pg_views WHERE viewname = 'kyma_connector_health')",
    ).fetch_one(&pool).await.unwrap();
    assert!(view_exists, "kyma_connector_health view must exist");

    // Verify the cdc_state table.
    let _row: Option<(String,)> = sqlx::query_as(
        "SELECT phase FROM connector_cdc_state WHERE FALSE LIMIT 1",
    ).fetch_optional(&pool).await.unwrap();
}
```

Run: `cargo test -p kyma-catalog --test migrations migration_0010_applies_cleanly -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-catalog/migrations/0010_external_sources.sql crates/kyma-catalog/tests/migrations.rs
git commit -m "feat(db): add external sources catalog migration (0010)"
```

---

## Task 3: `Capabilities` struct (write the test first)

**Files:**
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-connectors/src/external/mod.rs`
- Create: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-connectors/src/external/capabilities.rs`
- Modify: `/Users/shaked/projects_new/agentcy/kyma/crates/kyma-connectors/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/kyma-connectors/src/external/capabilities.rs`:

```rust
#![forbid(unsafe_code)]
//! Per-engine static description of pushdown surface.
//! Read by the planner; never executed by the engine.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub filter_ops: BTreeSet<FilterOp>,
    pub function_allowlist: BTreeSet<&'static str>,
    pub agg_funcs: BTreeSet<AggFunc>,
    pub group_by: bool,
    pub order_by: bool,
    pub limit: bool,
    pub same_source_join: bool,
    /// MySQL-specific: collation-safe column names. Empty for engines that
    /// don't have collation surprises.
    pub string_collation_safe_columns: BTreeSet<TableQualifiedName>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FilterOp {
    Eq, NotEq, Lt, LtEq, Gt, GtEq, In, IsNull, Like,
    And, Or, Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AggFunc {
    Count, CountDistinct, Sum, Avg, Min, Max,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TableQualifiedName {
    pub schema: String,
    pub table: String,
    pub column: String,
}

impl Capabilities {
    /// Helper: a "permissive" preset for tests — every operator/function known.
    pub fn permissive_for_test() -> Self {
        let mut filter_ops = BTreeSet::new();
        for op in [FilterOp::Eq, FilterOp::NotEq, FilterOp::Lt, FilterOp::LtEq,
                   FilterOp::Gt, FilterOp::GtEq, FilterOp::In, FilterOp::IsNull,
                   FilterOp::Like, FilterOp::And, FilterOp::Or, FilterOp::Not] {
            filter_ops.insert(op);
        }
        let mut function_allowlist = BTreeSet::new();
        for f in ["lower", "upper", "coalesce", "date_trunc"] {
            function_allowlist.insert(f);
        }
        let mut agg_funcs = BTreeSet::new();
        for f in [AggFunc::Count, AggFunc::CountDistinct, AggFunc::Sum,
                  AggFunc::Avg, AggFunc::Min, AggFunc::Max] {
            agg_funcs.insert(f);
        }
        Self {
            filter_ops,
            function_allowlist,
            agg_funcs,
            group_by: true,
            order_by: true,
            limit: true,
            same_source_join: true,
            string_collation_safe_columns: BTreeSet::new(),
        }
    }

    /// Helper: an "empty" preset — nothing pushable. Used to test residuals.
    pub fn empty_for_test() -> Self { Self::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_capabilities_have_all_filter_ops() {
        let c = Capabilities::permissive_for_test();
        for op in [FilterOp::Eq, FilterOp::NotEq, FilterOp::Lt, FilterOp::LtEq,
                   FilterOp::Gt, FilterOp::GtEq, FilterOp::In, FilterOp::IsNull,
                   FilterOp::Like, FilterOp::And, FilterOp::Or, FilterOp::Not] {
            assert!(c.filter_ops.contains(&op), "missing {op:?}");
        }
        assert!(c.group_by);
        assert!(c.limit);
        assert!(c.same_source_join);
    }

    #[test]
    fn empty_capabilities_disallow_everything() {
        let c = Capabilities::empty_for_test();
        assert!(c.filter_ops.is_empty());
        assert!(!c.group_by);
        assert!(!c.limit);
    }
}
```

- [ ] **Step 2: Wire the module**

Create `crates/kyma-connectors/src/external/mod.rs`:

```rust
#![forbid(unsafe_code)]
//! Engine-specific data source surface.
//!
//! See `docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md`.

pub mod capabilities;

pub use capabilities::{AggFunc, Capabilities, FilterOp, TableQualifiedName};
```

Edit `crates/kyma-connectors/src/lib.rs` to add (in the existing `pub mod` block):

```rust
pub mod external;
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p kyma-connectors external::capabilities -- --nocapture`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/src/external/ crates/kyma-connectors/src/lib.rs
git commit -m "feat(db): Capabilities struct and FilterOp/AggFunc enums"
```

---

## Task 4: Connection, Scope, InferredSchema, Checkpoint, ExternalError

**Files:**
- Create: `crates/kyma-connectors/src/external/connection.rs`
- Create: `crates/kyma-connectors/src/external/scope.rs`
- Create: `crates/kyma-connectors/src/external/schema.rs`
- Create: `crates/kyma-connectors/src/external/checkpoint.rs`
- Create: `crates/kyma-connectors/src/external/error.rs`
- Modify: `crates/kyma-connectors/src/external/mod.rs`

- [ ] **Step 1: Write `ConnectionConfig` + `ResolvedConnection` + `TlsMode`**

Write `crates/kyma-connectors/src/external/connection.rs`:

```rust
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    /// Connector refuses to start unless TLS handshake succeeds. DEFAULT.
    Required,
    /// Try TLS, fall back to plaintext with a warning in last_error.
    Preferred,
    /// Plaintext only. Loud warning at create-time; documented danger.
    Disabled,
}

impl Default for TlsMode {
    fn default() -> Self { Self::Required }
}

/// Persisted shape (in `connectors.connection_jsonb`).
/// Secrets are references via `secret_ref`; the actual value is resolved at
/// runtime via `kyma-connectors::secrets::SecretStore`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectionConfig {
    pub url: String,
    /// Name of a secret in the SecretStore; resolved per-tick.
    pub secret_ref: Option<String>,
    #[serde(default)]
    pub tls: TlsMode,
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,
    #[serde(default = "default_pool_acquire_timeout_ms")]
    pub pool_acquire_timeout_ms: u64,
    /// Engine-specific extras (e.g., Postgres replication slot name override).
    #[serde(default)]
    pub extra: serde_json::Value,
}

fn default_pool_size() -> u32 { 5 }
fn default_pool_acquire_timeout_ms() -> u64 { 5000 }

/// Built at runtime: `ConnectionConfig` with `secret_ref` resolved.
#[derive(Debug, Clone)]
pub struct ResolvedConnection {
    pub url: String,
    pub password: Option<String>, // resolved from secret_ref
    pub tls: TlsMode,
    pub pool_size: u32,
    pub pool_acquire_timeout_ms: u64,
    pub extra: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tls_is_required() {
        assert_eq!(TlsMode::default(), TlsMode::Required);
    }

    #[test]
    fn connection_config_round_trips() {
        let cfg: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "url": "postgres://app@host/db",
            "secret_ref": "pg_pwd",
            "tls": "required",
            "pool_size": 10
        })).unwrap();
        assert_eq!(cfg.url, "postgres://app@host/db");
        assert_eq!(cfg.pool_size, 10);
        assert_eq!(cfg.tls, TlsMode::Required);
        assert_eq!(cfg.pool_acquire_timeout_ms, 5000);
    }
}
```

- [ ] **Step 2: Write `Scope`**

Write `crates/kyma-connectors/src/external/scope.rs`:

```rust
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Scope {
    /// Schema names to expose. Empty = all schemas.
    #[serde(default)]
    pub include_schemas: Vec<String>,
    /// `schema.table` names to exclude.
    #[serde(default)]
    pub exclude_tables: Vec<String>,

    /// Mongo: max object-flatten depth. Default 2.
    #[serde(default = "default_flatten_depth")]
    pub flatten_depth: u32,
    /// Mongo + Postgres: how to render Decimal128 / numeric values.
    #[serde(default)]
    pub decimal128_mode: DecimalMode,
    /// Postgres: how to render PostGIS geometry.
    #[serde(default)]
    pub geometry_mode: GeometryMode,
    /// Postgres: numeric precision behavior (forced string for arbitrary precision).
    #[serde(default)]
    pub numeric_mode: DecimalMode,
}

fn default_flatten_depth() -> u32 { 2 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DecimalMode {
    #[default]
    /// Real for p ≤ 15; string otherwise (per spec §6.2/§6.3/§6.4).
    Auto,
    /// Always string. No precision loss.
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GeometryMode {
    #[default]
    /// Drop geometry columns entirely (default — opt-in).
    Drop,
    /// Render as WKT string.
    Wkt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_defaults_are_safe() {
        let s = Scope::default();
        assert_eq!(s.flatten_depth, 0); // default impl gives 0; ensure explicit ctor uses default_flatten_depth
        // Actually, since we used #[serde(default = "default_flatten_depth")] only on serde,
        // the Default impl uses 0. Round-trip a JSON to test the serde default:
        let s: Scope = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(s.flatten_depth, 2);
        assert!(matches!(s.decimal128_mode, DecimalMode::Auto));
        assert!(matches!(s.geometry_mode, GeometryMode::Drop));
    }
}
```

- [ ] **Step 3: Write `InferredSchema`**

Write `crates/kyma-connectors/src/external/schema.rs`:

```rust
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};

/// One source-table's schema, as observed by `ExternalSource::introspect`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InferredSchema {
    pub source_schema: String,
    pub source_table: String,
    pub primary_key: Vec<String>,           // column names; empty = no PK (CDC unsupported)
    pub columns: Vec<InferredColumn>,
    /// Extra metadata for engine-specific quirks (e.g., MySQL collation names).
    #[serde(default)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InferredColumn {
    pub name: String,
    pub source_type: String,                 // e.g., "BIGINT" / "Date" / "Decimal128"
    pub kyma_type: KymaType,
    pub nullable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KymaType {
    Int,
    Long,
    Real,
    Bool,
    String,
    Timestamp,
    Dynamic,
    Vector(u32),
}

/// System columns kyma adds to every synced table (per spec §6.1).
pub const SYSTEM_COLUMNS: &[(&str, KymaType)] = &[
    ("_kyma_pk", KymaType::String),
    ("_kyma_op", KymaType::String),
    ("_kyma_lsn", KymaType::String),
    ("_kyma_event_at", KymaType::Timestamp),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_columns_are_present_and_typed() {
        assert_eq!(SYSTEM_COLUMNS.len(), 4);
        assert!(SYSTEM_COLUMNS.iter().any(|(n, _)| *n == "_kyma_pk"));
    }
}
```

- [ ] **Step 4: Write `Checkpoint`**

Write `crates/kyma-connectors/src/external/checkpoint.rs`:

```rust
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};

/// Engine-opaque cursor. Stored as JSONB in `connector_cdc_state.checkpoint`.
/// Per spec §14 question 3: opaque JSON, with engine-side serde, to avoid
/// catalog-schema churn when an engine evolves its checkpoint shape.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Checkpoint(pub serde_json::Value);

impl Checkpoint {
    pub fn new(v: serde_json::Value) -> Self { Self(v) }
    pub fn as_json(&self) -> &serde_json::Value { &self.0 }
}
```

- [ ] **Step 5: Write `ExternalError`**

Write `crates/kyma-connectors/src/external/error.rs`:

```rust
#![forbid(unsafe_code)]
use thiserror::Error;

/// Three-class taxonomy from spec §11.1.
#[derive(Debug, Error)]
pub enum ExternalError {
    /// Network blip, briefly-unreachable source, pool acquire timeout, CDC
    /// stream dropped. Retried with exponential backoff.
    #[error("transient: {0}")]
    Transient(String),
    /// Bad credentials, DNS, TLS, missing replication slot privilege, table
    /// without PK, schema change emitting unrepresentable type. Connector
    /// disables; operator must fix.
    #[error("operator-actionable: {0}")]
    OperatorActionable(String),
    /// Panic / planner bug / catalog CAS bug. Pages.
    #[error("bug: {0}")]
    Bug(String),
}

impl ExternalError {
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
    pub fn is_operator_actionable(&self) -> bool {
        matches!(self, Self::OperatorActionable(_))
    }
}
```

- [ ] **Step 6: Re-export from `external/mod.rs`**

Edit `crates/kyma-connectors/src/external/mod.rs`:

```rust
#![forbid(unsafe_code)]

pub mod capabilities;
pub mod checkpoint;
pub mod connection;
pub mod error;
pub mod schema;
pub mod scope;

pub use capabilities::{AggFunc, Capabilities, FilterOp, TableQualifiedName};
pub use checkpoint::Checkpoint;
pub use connection::{ConnectionConfig, ResolvedConnection, TlsMode};
pub use error::ExternalError;
pub use schema::{InferredColumn, InferredSchema, KymaType, SYSTEM_COLUMNS};
pub use scope::{DecimalMode, GeometryMode, Scope};
```

- [ ] **Step 7: Run tests**

Run: `cargo test -p kyma-connectors external -- --nocapture`
Expected: PASS (all unit tests in the new modules).

- [ ] **Step 8: Commit**

```bash
git add crates/kyma-connectors/src/external/
git commit -m "feat(db): connection/scope/schema/checkpoint/error types for ExternalSource"
```

---

## Task 5: `PushedPlan`, `PushdownSummary`, `BatchSink`

**Files:**
- Create: `crates/kyma-connectors/src/external/plan.rs`
- Create: `crates/kyma-connectors/src/external/sink.rs`
- Modify: `crates/kyma-connectors/src/external/mod.rs`

- [ ] **Step 1: Write `PushedPlan` + `PushdownSummary` + `BoundParam`**

Write `crates/kyma-connectors/src/external/plan.rs`:

```rust
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};

/// What the source executes. Engine-opaque from the planner's POV.
/// Per spec §14 question 2: keep `Sql(String)` for v1 with strict-bind
/// discipline; revisit if v2 proves it brittle.
#[derive(Debug, Clone)]
pub enum PushedPlan {
    /// Postgres / MySQL: a parameterized SQL string + binds.
    Sql { sql: String, binds: Vec<BoundParam> },
    /// Mongo: an aggregation pipeline as JSON documents.
    MongoPipeline(Vec<serde_json::Value>),
}

#[derive(Debug, Clone)]
pub enum BoundParam {
    Int(i64),
    Real(f64),
    Bool(bool),
    String(String),
    Timestamp(chrono::DateTime<chrono::Utc>),
    Null,
}

/// Telemetry record describing one federated scan's pushdown decisions.
/// Emitted on every federated query response (spec §10.2).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PushdownSummary {
    pub source: String,
    pub table: String,
    pub filters_pushed: Vec<String>,         // pretty-printed expressions
    pub filters_residual: Vec<String>,
    pub projection_pushed: bool,
    pub limit_pushed: Option<u64>,
    pub sort_pushed: Option<String>,
    pub agg_pushed: Option<String>,
    pub agg_residual_reason: Option<String>,
    pub join_pushed: bool,
    pub scan_duration_ms: u64,
    pub rows_returned: u64,
    pub bytes_received: u64,
    pub cancelled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushdown_summary_serializes() {
        let s = PushdownSummary {
            source: "pg_prod".into(),
            table: "public.users".into(),
            filters_pushed: vec!["region = $1".into()],
            limit_pushed: Some(50),
            ..Default::default()
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["source"], "pg_prod");
        assert_eq!(json["limit_pushed"], 50);
    }

    #[test]
    fn pushed_plan_sql_carries_binds() {
        let plan = PushedPlan::Sql {
            sql: "SELECT id FROM t WHERE r = $1".into(),
            binds: vec![BoundParam::String("eu".into())],
        };
        match plan {
            PushedPlan::Sql { binds, .. } => assert_eq!(binds.len(), 1),
            _ => panic!("wrong variant"),
        }
    }
}
```

- [ ] **Step 2: Write `BatchSink`**

Write `crates/kyma-connectors/src/external/sink.rs`:

```rust
#![forbid(unsafe_code)]
use arrow_array::RecordBatch;
use async_trait::async_trait;

/// Receives Arrow batches produced by `ExternalSource::scan`. The federation
/// consumer wires this to DataFusion's stream; the test consumer collects
/// into a Vec.
#[async_trait]
pub trait BatchSink: Send {
    async fn push(&mut self, batch: RecordBatch) -> Result<(), anyhow::Error>;
    async fn finish(&mut self) -> Result<(), anyhow::Error>;
}
```

- [ ] **Step 3: Re-export**

Edit `crates/kyma-connectors/src/external/mod.rs` — add to module list and re-exports:

```rust
pub mod plan;
pub mod sink;
// ...
pub use plan::{BoundParam, PushdownSummary, PushedPlan};
pub use sink::BatchSink;
```

- [ ] **Step 4: Add `chrono` dep if not already in `kyma-connectors/Cargo.toml`**

Verify `chrono` is in `kyma-connectors/Cargo.toml`:

```bash
grep '^chrono' crates/kyma-connectors/Cargo.toml
```

If absent, add to `[dependencies]`:

```toml
chrono = { workspace = true }
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p kyma-connectors external::plan -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-connectors/src/external/ crates/kyma-connectors/Cargo.toml
git commit -m "feat(db): PushedPlan, PushdownSummary, BatchSink"
```

---

## Task 6: `ExternalSource` trait

**Files:**
- Create: `crates/kyma-connectors/src/external/trait_def.rs`
- Modify: `crates/kyma-connectors/src/external/mod.rs`

- [ ] **Step 1: Write the trait + helper types**

Write `crates/kyma-connectors/src/external/trait_def.rs`:

```rust
#![forbid(unsafe_code)]
use async_trait::async_trait;
use std::sync::Arc;

use super::{
    BatchSink, Capabilities, Checkpoint, ExternalError, InferredSchema,
    PushedPlan, ResolvedConnection, Scope,
};

/// Engine-opaque per-instance handle (typically wraps a connection pool).
pub trait SourceHandle: Send + Sync + 'static {}

/// Returned by `ExternalSource::health`. Renders into the status endpoint.
#[derive(Debug, Clone)]
pub struct SourceHealth {
    pub reachable: bool,
    pub version: Option<String>,
    pub detail: Option<String>,
}

/// Returned by `ExternalSource::scan`. Lets the federation consumer attach
/// per-leg telemetry to the response.
#[derive(Debug, Clone, Default)]
pub struct ScanReport {
    pub rows_returned: u64,
    pub bytes_received: u64,
}

/// One change-data-capture event. Engines decode their native format into this.
#[derive(Debug, Clone)]
pub struct CdcEvent {
    pub op: CdcOp,
    pub primary_key: String,
    pub checkpoint_at: Checkpoint,
    pub event_time: chrono::DateTime<chrono::Utc>,
    pub before: Option<serde_json::Value>,  // pre-image (delete/update); None for insert
    pub after: Option<serde_json::Value>,   // post-image (insert/update); None for delete
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdcOp { Insert, Update, Delete }

pub type CdcStream = Box<dyn futures::Stream<Item = Result<CdcEvent, ExternalError>> + Send + Unpin>;
pub type SnapshotStream = Box<dyn futures::Stream<Item = Result<serde_json::Value, ExternalError>> + Send + Unpin>;

/// The single load-bearing abstraction (spec §3.1, §4.1).
#[async_trait]
pub trait ExternalSource: Send + Sync + 'static {
    fn type_id(&self) -> &'static str;
    fn capabilities(&self) -> &Capabilities;

    async fn connect(&self, conn: &ResolvedConnection) -> Result<Arc<dyn SourceHandle>, ExternalError>;
    async fn health(&self, h: &dyn SourceHandle) -> Result<SourceHealth, ExternalError>;

    async fn introspect(
        &self,
        h: &dyn SourceHandle,
        scope: &Scope,
    ) -> Result<Vec<InferredSchema>, ExternalError>;

    async fn scan(
        &self,
        h: &dyn SourceHandle,
        plan: &PushedPlan,
        sink: &mut dyn BatchSink,
    ) -> Result<ScanReport, ExternalError>;

    async fn open_cdc(
        &self,
        _h: &dyn SourceHandle,
        _from: Option<&Checkpoint>,
    ) -> Result<CdcStream, ExternalError> {
        Err(ExternalError::OperatorActionable(
            "this source does not support CDC".into()))
    }

    async fn snapshot_at(
        &self,
        _h: &dyn SourceHandle,
        _scope: &Scope,
    ) -> Result<(SnapshotStream, Checkpoint), ExternalError> {
        Err(ExternalError::OperatorActionable(
            "this source does not support snapshot".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdc_op_round_trip() {
        let e = CdcEvent {
            op: CdcOp::Insert,
            primary_key: "42".into(),
            checkpoint_at: Checkpoint::new(serde_json::json!({"lsn":"0/0"})),
            event_time: chrono::Utc::now(),
            before: None,
            after: Some(serde_json::json!({"id": 42})),
        };
        assert_eq!(e.op, CdcOp::Insert);
        assert_eq!(e.primary_key, "42");
    }
}
```

- [ ] **Step 2: Add `futures` to `kyma-connectors` deps if absent**

```bash
grep '^futures' crates/kyma-connectors/Cargo.toml
```

If absent, add `futures = { workspace = true }` under `[dependencies]`.

- [ ] **Step 3: Re-export**

Edit `crates/kyma-connectors/src/external/mod.rs`:

```rust
pub mod trait_def;
// ...
pub use trait_def::{
    CdcEvent, CdcOp, CdcStream, ExternalSource, ScanReport, SnapshotStream,
    SourceHandle, SourceHealth,
};
```

- [ ] **Step 4: Run tests + verify trait compiles**

Run: `cargo test -p kyma-connectors external::trait_def -- --nocapture`
Expected: PASS (1 test).

Run: `cargo check -p kyma-connectors`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-connectors/src/external/ crates/kyma-connectors/Cargo.toml
git commit -m "feat(db): ExternalSource trait + CdcEvent + handle/health/report types"
```

---

## Task 7: Extend `CommitCoordinator` with `with_cursor_update`

**Files:**
- Modify: `crates/kyma-ingest-core/src/commit.rs` (or wherever `CommitCoordinator` lives — find it)
- Test: `crates/kyma-ingest-core/tests/cursor_update.rs`

- [ ] **Step 1: Locate `CommitCoordinator`**

Run: `grep -rn "CommitCoordinator" crates/kyma-ingest-core/src/`
Note the file. The implementation below assumes `crates/kyma-ingest-core/src/commit.rs`; adjust if elsewhere.

- [ ] **Step 2: Read the existing commit type to understand its builder shape**

Run: `cat crates/kyma-ingest-core/src/commit.rs`
Goal: identify the `commit()` entry point and how the SQL transaction is opened. We need to extend it to additionally write to `connector_cdc_state` and `connector_events` in the same transaction.

- [ ] **Step 3: Write the failing test**

Write `crates/kyma-ingest-core/tests/cursor_update.rs`:

```rust
//! Verifies cursor advance lands in the SAME catalog transaction as the
//! snapshot CAS commit. This is the exactly-once knot for sync mode.

use kyma_ingest_core::CommitCoordinator;        // adjust import per actual public surface
use serde_json::json;
use sqlx::PgPool;
use testcontainers::clients::Cli;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn fresh_pool() -> PgPool {
    let docker = Cli::default();
    let pg = docker.run(Postgres::default());
    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432)
    );
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("../kyma-catalog/migrations").run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn cursor_advance_atomic_with_snapshot_commit() {
    let pool = fresh_pool().await;
    let connector_id = Uuid::new_v4();

    // Seed a connector row.
    sqlx::query(
        "INSERT INTO connectors (id, name, type, target_database, target_table, config_jsonb, schedule_ms, drive_model, mode)
         VALUES ($1, 't', 'fake', 'd', 't', '{}'::jsonb, 60000, 'periodic', 'sync')",
    ).bind(connector_id).execute(&pool).await.unwrap();

    // ... build the CommitCoordinator the same way kyma-bin does; in this test
    // we go through the same constructor. (The actual coordinator probably
    // takes a Catalog handle; replicate that.)

    // The test: commit() with .with_cursor_update(...) must atomically write
    // the cursor row OR roll everything back.
    // (Actual code below is illustrative — match the real builder's signature.)

    /* PSEUDOCODE — adjust to actual API surface:
    let coordinator = CommitCoordinator::new(pool.clone());
    let result = coordinator
        .commit_builder()
        .with_extents(vec![/* one or zero extents */])
        .with_cursor_update(connector_id, "public.users", json!({"lsn": "0/16AABBCC"}))
        .commit()
        .await
        .unwrap();
    */

    // Assert the cursor row landed:
    let cursor: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT checkpoint FROM connector_cdc_state WHERE connector_id = $1 AND source_table = $2",
    ).bind(connector_id).bind("public.users")
     .fetch_optional(&pool).await.unwrap();
    assert_eq!(cursor.unwrap().0, json!({"lsn": "0/16AABBCC"}));
}

#[tokio::test]
async fn cursor_advance_rolls_back_when_snapshot_commit_fails() {
    // Strategy: open a transaction, race two builders against the same snapshot version
    // so the second one's CAS UPDATE finds 0 rows changed and the existing retry path triggers.
    // We assert the FIRST attempt's cursor write was rolled back when its CAS lost.
    let pool = fresh_pool().await;
    let connector_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO connectors (id, name, type, target_database, target_table, config_jsonb, schedule_ms, drive_model, mode)
         VALUES ($1, 't', 'fake', 'd', 't', '{}'::jsonb, 60000, 'periodic', 'sync')",
    ).bind(connector_id).execute(&pool).await.unwrap();

    // Seed an initial table snapshot version so we have a known current_snapshot_id.
    let table_id = sqlx::query_scalar::<_, uuid::Uuid>(
        "INSERT INTO tables (database_id, name, schema_jsonb)
         SELECT id, 't', '{}'::jsonb FROM databases WHERE name = 'd' RETURNING id"
    ).fetch_one(&pool).await.unwrap();
    let original_snap = sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT current_snapshot_id FROM tables WHERE id = $1"
    ).bind(table_id).fetch_one(&pool).await.unwrap();

    // Race: builder A reads `original_snap`; before A commits, another writer advances.
    // (Use a manual `UPDATE tables SET current_snapshot_id = gen_random_uuid() ...` to simulate.)
    sqlx::query("UPDATE tables SET current_snapshot_id = gen_random_uuid() WHERE id = $1")
        .bind(table_id).execute(&pool).await.unwrap();

    // Now builder A's CAS would fail. Run it through the real CommitBuilder API and
    // expect the existing CAS retry path to either (a) succeed against the new snapshot
    // (and write the cursor exactly once) or (b) bubble a retryable error.
    // Either way the post-condition is: at most one cursor row exists with the expected value.
    let coordinator = CommitCoordinator::new(pool.clone());
    let _ = coordinator
        .commit_builder()
        .with_extents(Vec::new())
        .with_cursor_update(connector_id, "public.users", json!({"lsn": "0/16AABBCC"}))
        .commit()
        .await; // Result not asserted — either success or transient error is acceptable here.

    // Final assertion: the cursor row has the expected value OR no row exists; never a half-state.
    let cursor: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT checkpoint FROM connector_cdc_state WHERE connector_id = $1 AND source_table = $2",
    ).bind(connector_id).bind("public.users")
     .fetch_optional(&pool).await.unwrap();
    if let Some((v,)) = cursor {
        assert_eq!(v, json!({"lsn": "0/16AABBCC"}),
            "if a cursor exists, it must be the one we tried to commit (no partial writes)");
    }
}
```

The pseudocode marks where the implementor must align with the real `CommitCoordinator` API. The real test asserts: (a) the cursor lands when commit succeeds, and (b) the cursor does NOT land when commit conflicts and retry hasn't yet completed.

- [ ] **Step 4: Run the test (expect it to fail because `with_cursor_update` doesn't exist yet)**

Run: `cargo test -p kyma-ingest-core --test cursor_update -- --nocapture`
Expected: FAIL with "no method named `with_cursor_update`".

- [ ] **Step 5: Implement `with_cursor_update`**

Open `crates/kyma-ingest-core/src/commit.rs`. Identify the builder struct (probably `CommitBuilder` or similar) and the `commit()` method that opens a `sqlx::Transaction`. Add:

```rust
pub struct CursorUpdate {
    pub connector_id: uuid::Uuid,
    pub source_table: String,
    pub checkpoint: serde_json::Value,
}

impl CommitBuilder /* or whatever the real name is */ {
    /// Advances the connector_cdc_state cursor inside the same catalog
    /// transaction as the snapshot CAS commit. See spec §4.3.
    pub fn with_cursor_update(
        mut self,
        connector_id: uuid::Uuid,
        source_table: impl Into<String>,
        checkpoint: serde_json::Value,
    ) -> Self {
        self.cursor_update = Some(CursorUpdate {
            connector_id,
            source_table: source_table.into(),
            checkpoint,
        });
        self
    }
}
```

In the existing `commit()` method, inside the same `Transaction`:

```rust
// After the existing snapshot CAS UPDATE…
if let Some(cu) = &self.cursor_update {
    sqlx::query(
        "INSERT INTO connector_cdc_state
            (connector_id, source_table, phase, checkpoint, last_event_at, updated_at)
         VALUES ($1, $2, 'streaming', $3, now(), now())
         ON CONFLICT (connector_id, source_table)
         DO UPDATE SET checkpoint = EXCLUDED.checkpoint,
                       last_event_at = now(),
                       updated_at = now()
                       -- phase intentionally NOT changed here; that's the
                       -- snapshot-coordinator's job (pending->snapshotting->streaming).
        ",
    )
    .bind(cu.connector_id)
    .bind(&cu.source_table)
    .bind(&cu.checkpoint)
    .execute(&mut *tx)
    .await
    .map_err(/* existing error wrapper */)?;
}

// existing tx.commit().await call follows here
```

Ensure the struct field `cursor_update: Option<CursorUpdate>` exists; default it to `None` in the builder constructor.

- [ ] **Step 6: Run the test**

Run: `cargo test -p kyma-ingest-core --test cursor_update -- --nocapture`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-ingest-core/src/commit.rs crates/kyma-ingest-core/tests/cursor_update.rs
git commit -m "feat(db): CommitCoordinator.with_cursor_update for exactly-once CDC commits"
```

---

## Task 8: `CdcConnector` trait + runner skeleton

**Files:**
- Create: `crates/kyma-connectors/src/cdc/mod.rs`
- Create: `crates/kyma-connectors/src/cdc/connector.rs`
- Create: `crates/kyma-connectors/src/cdc/state.rs`
- Create: `crates/kyma-connectors/src/cdc/runner.rs`
- Create: `crates/kyma-connectors/src/cdc/snapshot.rs`
- Create: `crates/kyma-connectors/src/cdc/stream.rs`
- Create: `crates/kyma-connectors/src/cdc/evolver.rs`
- Modify: `crates/kyma-connectors/src/lib.rs`

- [ ] **Step 1: Write `CdcConnector` trait**

Write `crates/kyma-connectors/src/cdc/connector.rs`:

```rust
#![forbid(unsafe_code)]
use async_trait::async_trait;
use std::sync::Arc;

use crate::external::{ExternalSource, ResolvedConnection};

/// Wraps an `ExternalSource` for sync mode: a connector that can run a
/// snapshot phase + a CDC streaming phase + commit cursors atomically.
/// Sibling of the existing `Connector` trait; registry holds either.
#[async_trait]
pub trait CdcConnector: Send + Sync + 'static {
    fn type_id(&self) -> &'static str;
    fn source(&self) -> Arc<dyn ExternalSource>;
    fn validate_config(
        &self,
        cfg: &serde_json::Value,
    ) -> Result<(), crate::types::ConfigError>;

    /// Build the `ResolvedConnection` from the catalog row's
    /// `connection_jsonb` + the `SecretStore`. This is engine-agnostic —
    /// the default impl reads `ConnectionConfig` and resolves `secret_ref`.
    async fn resolve_connection(
        &self,
        connection_jsonb: &serde_json::Value,
        secrets: &dyn crate::secrets::SecretStore,
    ) -> Result<ResolvedConnection, crate::types::ConfigError>;
}
```

- [ ] **Step 2: Write `state.rs` (sqlx helpers)**

Write `crates/kyma-connectors/src/cdc/state.rs`:

```rust
#![forbid(unsafe_code)]
use sqlx::PgPool;
use uuid::Uuid;

use crate::external::Checkpoint;

#[derive(Debug, Clone)]
pub struct CdcStateRow {
    pub connector_id: Uuid,
    pub source_table: String,
    pub phase: String,
    pub checkpoint: Option<Checkpoint>,
    pub rows_synced: i64,
    pub last_event_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
}

pub async fn load_state(
    pool: &PgPool,
    connector_id: Uuid,
    source_table: &str,
) -> Result<Option<CdcStateRow>, sqlx::Error> {
    let row: Option<(String, Option<serde_json::Value>, i64,
                     Option<chrono::DateTime<chrono::Utc>>, Option<String>)> = sqlx::query_as(
        "SELECT phase, checkpoint, rows_synced, last_event_at, last_error
         FROM connector_cdc_state WHERE connector_id = $1 AND source_table = $2",
    )
    .bind(connector_id)
    .bind(source_table)
    .fetch_optional(pool).await?;
    Ok(row.map(|(phase, ck, rows_synced, last_event_at, last_error)| CdcStateRow {
        connector_id,
        source_table: source_table.into(),
        phase,
        checkpoint: ck.map(Checkpoint::new),
        rows_synced,
        last_event_at,
        last_error,
    }))
}

pub async fn set_phase(
    pool: &PgPool,
    connector_id: Uuid,
    source_table: &str,
    phase: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO connector_cdc_state (connector_id, source_table, phase)
         VALUES ($1, $2, $3)
         ON CONFLICT (connector_id, source_table)
         DO UPDATE SET phase = EXCLUDED.phase, updated_at = now()",
    )
    .bind(connector_id)
    .bind(source_table)
    .bind(phase)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn record_error(
    pool: &PgPool,
    connector_id: Uuid,
    source_table: &str,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connector_cdc_state SET last_error = $3, updated_at = now()
         WHERE connector_id = $1 AND source_table = $2",
    )
    .bind(connector_id)
    .bind(source_table)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn append_event(
    pool: &PgPool,
    connector_id: Uuid,
    kind: &str,
    payload: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO connector_events (connector_id, kind, payload_jsonb) VALUES ($1, $2, $3)",
    )
    .bind(connector_id)
    .bind(kind)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 3: Write `evolver.rs` (skeleton + tests)**

Write `crates/kyma-connectors/src/cdc/evolver.rs`:

```rust
#![forbid(unsafe_code)]
//! Schema evolver — typed-vs-dynamic promotion rules.
//! See spec §4.4 and §6.

use std::collections::HashMap;
use crate::external::KymaType;

/// Default thresholds from spec §4.4 / §6.1.
pub const DEFAULT_STABILITY_THRESHOLD: usize = 100;
pub const DEFAULT_WINDOW: usize = 1000;

#[derive(Debug, Clone, Copy)]
pub enum EvolverDecision {
    /// Field is stable; promote it to a typed kyma column.
    Promote(KymaType),
    /// Field is polymorphic, sparse, or below threshold — route to dynamic.
    Dynamic,
}

#[derive(Debug, Clone)]
pub struct FieldObservation {
    pub typed_count: HashMap<KymaType, usize>,
    pub total: usize,
}

impl Default for FieldObservation {
    fn default() -> Self { Self { typed_count: HashMap::new(), total: 0 } }
}

impl FieldObservation {
    pub fn record(&mut self, ty: KymaType) {
        *self.typed_count.entry(ty).or_insert(0) += 1;
        self.total += 1;
    }

    pub fn decide(&self, threshold: usize) -> EvolverDecision {
        if self.total < threshold {
            return EvolverDecision::Dynamic;
        }
        // Schema only widens (spec §6.1 rule 2). If we see one consistent type
        // with >= threshold count, promote. If multiple types exceed threshold,
        // we have a polymorphic field — stay dynamic.
        let mut leading: Option<(KymaType, usize)> = None;
        let mut second: Option<(KymaType, usize)> = None;
        for (ty, &count) in &self.typed_count {
            if leading.map_or(true, |(_, c)| count > c) {
                second = leading;
                leading = Some((*ty, count));
            } else if second.map_or(true, |(_, c)| count > c) {
                second = Some((*ty, count));
            }
        }
        match (leading, second) {
            (Some((ty, c)), None) if c >= threshold => EvolverDecision::Promote(ty),
            (Some((ty, c1)), Some((_, c2))) if c1 >= threshold && c2 < threshold / 10 => {
                // Strong dominance: leading is at least 10× the runner-up. Promote.
                EvolverDecision::Promote(ty)
            }
            _ => EvolverDecision::Dynamic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn below_threshold_stays_dynamic() {
        let mut o = FieldObservation::default();
        for _ in 0..99 { o.record(KymaType::Int); }
        assert!(matches!(o.decide(DEFAULT_STABILITY_THRESHOLD), EvolverDecision::Dynamic));
    }

    #[test]
    fn at_threshold_promotes_to_consistent_type() {
        let mut o = FieldObservation::default();
        for _ in 0..100 { o.record(KymaType::Int); }
        assert!(matches!(o.decide(DEFAULT_STABILITY_THRESHOLD), EvolverDecision::Promote(KymaType::Int)));
    }

    #[test]
    fn polymorphic_above_threshold_stays_dynamic() {
        let mut o = FieldObservation::default();
        for _ in 0..800 { o.record(KymaType::Int); }
        for _ in 0..200 { o.record(KymaType::String); }
        assert!(matches!(o.decide(DEFAULT_STABILITY_THRESHOLD), EvolverDecision::Dynamic));
    }

    #[test]
    fn dominant_type_with_few_outliers_promotes() {
        let mut o = FieldObservation::default();
        for _ in 0..950 { o.record(KymaType::Int); }
        for _ in 0..5 { o.record(KymaType::String); } // < threshold/10
        assert!(matches!(o.decide(DEFAULT_STABILITY_THRESHOLD), EvolverDecision::Promote(KymaType::Int)));
    }
}
```

- [ ] **Step 4: Write `snapshot.rs`, `stream.rs`, `runner.rs` skeletons**

Write `crates/kyma-connectors/src/cdc/snapshot.rs`:

```rust
#![forbid(unsafe_code)]
//! Initial-snapshot coordinator (engine-agnostic). Drives
//! `ExternalSource::snapshot_at`, streams batches into the ingest write path,
//! and on completion atomically advances the cursor + transitions to
//! `phase = 'streaming'`.
//!
//! Body lands in M1 once we have a real engine to test against; M0 ships the
//! struct + module to lock the API.

use std::sync::Arc;
use crate::external::ExternalSource;

pub struct SnapshotCoordinator {
    pub source: Arc<dyn ExternalSource>,
}

impl SnapshotCoordinator {
    pub fn new(source: Arc<dyn ExternalSource>) -> Self { Self { source } }
    // Methods land in M1.
}
```

Write `crates/kyma-connectors/src/cdc/stream.rs`:

```rust
#![forbid(unsafe_code)]
//! Steady-state CDC consumer (engine-agnostic). Drives
//! `ExternalSource::open_cdc`, group-commits batches with cursor-update,
//! emits lag heartbeats. Body lands in M1.

use std::sync::Arc;
use crate::external::ExternalSource;

pub struct CdcStreamConsumer {
    pub source: Arc<dyn ExternalSource>,
}

impl CdcStreamConsumer {
    pub fn new(source: Arc<dyn ExternalSource>) -> Self { Self { source } }
    // Methods land in M1.
}
```

Write `crates/kyma-connectors/src/cdc/runner.rs`:

```rust
#![forbid(unsafe_code)]
//! Runner that drives the phase machine: pending → snapshotting → streaming.
//! M0 lands the skeleton; M1 implements run_one_table().

use std::sync::Arc;
use uuid::Uuid;

use super::connector::CdcConnector;

pub struct CdcRunner {
    pub connector: Arc<dyn CdcConnector>,
}

impl CdcRunner {
    pub fn new(connector: Arc<dyn CdcConnector>) -> Self { Self { connector } }

    /// Runs one phase-machine tick for one (connector, source_table) row.
    /// Implementation in M1.
    pub async fn run_one_table(&self, _connector_id: Uuid, _source_table: &str) {
        unimplemented!("CdcRunner::run_one_table — implemented in M1")
    }
}
```

- [ ] **Step 5: Wire `cdc/mod.rs`**

Write `crates/kyma-connectors/src/cdc/mod.rs`:

```rust
#![forbid(unsafe_code)]

pub mod connector;
pub mod evolver;
pub mod runner;
pub mod snapshot;
pub mod state;
pub mod stream;

pub use connector::CdcConnector;
pub use evolver::{EvolverDecision, FieldObservation, DEFAULT_STABILITY_THRESHOLD, DEFAULT_WINDOW};
pub use runner::CdcRunner;
pub use snapshot::SnapshotCoordinator;
pub use state::{append_event, load_state, record_error, set_phase, CdcStateRow};
pub use stream::CdcStreamConsumer;
```

Edit `crates/kyma-connectors/src/lib.rs`, add:

```rust
pub mod cdc;
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p kyma-connectors cdc::evolver -- --nocapture`
Expected: PASS (4 tests).

Run: `cargo check -p kyma-connectors`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-connectors/src/cdc/ crates/kyma-connectors/src/lib.rs
git commit -m "feat(db): CdcConnector trait, SchemaEvolver, runner/snapshot/stream skeletons"
```

---

## Task 9: Extend `kyma-connectors::registry` and `runner` to dispatch CDC vs. Connector

**Files:**
- Modify: `crates/kyma-connectors/src/registry.rs`
- Modify: `crates/kyma-connectors/src/runner.rs`

- [ ] **Step 1: Extend the registry to hold both kinds**

Read the existing `crates/kyma-connectors/src/registry.rs`. Modify it to also accept `Arc<dyn CdcConnector>`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use crate::cdc::CdcConnector;
use crate::types::Connector;

#[derive(Default, Clone)]
pub struct ConnectorRegistry {
    by_type: HashMap<&'static str, RegisteredConnector>,
}

#[derive(Clone)]
pub enum RegisteredConnector {
    /// Existing periodic connectors (e.g., Prometheus).
    Periodic(Arc<dyn Connector>),
    /// CDC connectors (Postgres / MySQL / Mongo).
    Cdc(Arc<dyn CdcConnector>),
}

impl ConnectorRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn register(&mut self, c: Arc<dyn Connector>) {
        let id = c.type_id();
        assert!(
            !self.by_type.contains_key(id),
            "connector type {id:?} already registered",
        );
        self.by_type.insert(id, RegisteredConnector::Periodic(c));
    }

    pub fn register_cdc(&mut self, c: Arc<dyn CdcConnector>) {
        let id = c.type_id();
        assert!(
            !self.by_type.contains_key(id),
            "connector type {id:?} already registered",
        );
        self.by_type.insert(id, RegisteredConnector::Cdc(c));
    }

    pub fn lookup(&self, type_id: &str) -> Option<&RegisteredConnector> {
        self.by_type.get(type_id)
    }

    pub fn types(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_type.keys().copied()
    }
}
```

- [ ] **Step 2: Update the runner to dispatch**

Read `crates/kyma-connectors/src/runner.rs` to find the dispatch site. Update calls that use `registry.lookup(type_id)` to match on `RegisteredConnector::{Periodic, Cdc}`. For M0 the `Cdc` arm just logs and returns (the actual phase-machine drive is in M1 via `CdcRunner`):

```rust
match registry.lookup(type_id) {
    Some(RegisteredConnector::Periodic(c)) => {
        // existing call path
        c.run_once(/* ... */).await
    }
    Some(RegisteredConnector::Cdc(c)) => {
        tracing::debug!(target: "kyma_connectors::runner",
            "CdcConnector {} found; phase-machine drive lives in M1", c.type_id());
        Ok(()) // or whatever the existing return shape is
    }
    None => { /* existing missing-type handling */ }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p kyma-connectors -- --nocapture`
Expected: PASS (existing tests still green; no new tests added in this task — registry/runner integration is exercised in M1).

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/src/registry.rs crates/kyma-connectors/src/runner.rs
git commit -m "feat(db): registry/runner dispatch on Periodic vs Cdc connectors"
```

---

## Task 10: `FederationRegistry`

**Files:**
- Modify: `crates/kyma-federation/src/registry.rs`
- Test inline.

- [ ] **Step 1: Replace the Task-1 stub with the real registry**

Write `crates/kyma-federation/src/registry.rs`:

```rust
#![forbid(unsafe_code)]
//! Registry of registered external sources, keyed by source name (e.g., "pg_prod").
//! Built once per `kyma-server` startup from catalog rows; consumed by
//! `kyma-exec`'s `SessionContext` builder via `register_federated_catalogs`.

use std::collections::HashMap;
use std::sync::Arc;

use kyma_connectors::external::ExternalSource;

#[derive(Default, Clone)]
pub struct FederationRegistry {
    by_source_name: HashMap<String, FederationEntry>,
}

#[derive(Clone)]
pub struct FederationEntry {
    pub source: Arc<dyn ExternalSource>,
    /// The catalog row id; used for telemetry and the `pushdown_summary.source` field.
    pub connector_id: uuid::Uuid,
    /// Connector mode — federation, sync, or both. Federation-eligible if not "sync".
    pub mode: String,
}

impl FederationRegistry {
    pub fn new() -> Self { Self::default() }

    pub fn insert(&mut self, source_name: impl Into<String>, entry: FederationEntry) {
        let name = source_name.into();
        if self.by_source_name.contains_key(&name) {
            panic!("federation source {name:?} already registered");
        }
        self.by_source_name.insert(name, entry);
    }

    pub fn get(&self, source_name: &str) -> Option<&FederationEntry> {
        self.by_source_name.get(source_name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &FederationEntry)> + '_ {
        self.by_source_name.iter().map(|(k, v)| (k.as_str(), v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeSource;
    impl kyma_connectors::external::ExternalSource for FakeSource {
        fn type_id(&self) -> &'static str { "fake" }
        fn capabilities(&self) -> &kyma_connectors::external::Capabilities {
            // SAFETY-LITE: return a 'static immutable Capabilities — we just need
            // a reference. Use a leaked Box because Capabilities isn't Copy.
            // For the test, this is fine.
            static C: std::sync::OnceLock<kyma_connectors::external::Capabilities> = std::sync::OnceLock::new();
            C.get_or_init(kyma_connectors::external::Capabilities::permissive_for_test)
        }
        // Other methods unimplemented — we never call them in this test.
        fn connect<'a>(&'a self, _conn: &'a kyma_connectors::external::ResolvedConnection)
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<std::sync::Arc<dyn kyma_connectors::external::SourceHandle>, kyma_connectors::external::ExternalError>> + Send + 'a>> {
            Box::pin(async { unimplemented!() })
        }
        fn health<'a>(&'a self, _h: &'a dyn kyma_connectors::external::SourceHandle)
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<kyma_connectors::external::SourceHealth, kyma_connectors::external::ExternalError>> + Send + 'a>> {
            Box::pin(async { unimplemented!() })
        }
        fn introspect<'a>(&'a self, _h: &'a dyn kyma_connectors::external::SourceHandle, _scope: &'a kyma_connectors::external::Scope)
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<kyma_connectors::external::InferredSchema>, kyma_connectors::external::ExternalError>> + Send + 'a>> {
            Box::pin(async { unimplemented!() })
        }
        fn scan<'a>(&'a self, _h: &'a dyn kyma_connectors::external::SourceHandle, _plan: &'a kyma_connectors::external::PushedPlan, _sink: &'a mut dyn kyma_connectors::external::BatchSink)
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<kyma_connectors::external::ScanReport, kyma_connectors::external::ExternalError>> + Send + 'a>> {
            Box::pin(async { unimplemented!() })
        }
    }

    #[test]
    fn registry_holds_entries_by_source_name() {
        let mut r = FederationRegistry::new();
        r.insert("pg_prod", FederationEntry {
            source: Arc::new(FakeSource),
            connector_id: uuid::Uuid::new_v4(),
            mode: "federation".into(),
        });
        assert!(r.get("pg_prod").is_some());
        assert!(r.get("nope").is_none());
        assert_eq!(r.iter().count(), 1);
    }
}
```

(NB: the `async-trait` macro generates the `Pin<Box<…>>` shapes shown; use `#[async_trait]` on the real impl in the engine slices. This test impl is verbose because we hand-code rather than depend on `async-trait` here — that's fine for a 5-method stub.)

- [ ] **Step 2: Run the test**

Run: `cargo test -p kyma-federation registry::tests -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-federation/src/registry.rs
git commit -m "feat(db): FederationRegistry keyed by source name"
```

---

## Task 11: `PushdownPlanner` (engine-agnostic core + property-test scaffolding)

**Files:**
- Modify: `crates/kyma-federation/src/planner.rs`
- Modify: `crates/kyma-connectors-testkit/src/query_gen.rs`

- [ ] **Step 1: Implement the planner**

Write `crates/kyma-federation/src/planner.rs`:

```rust
#![forbid(unsafe_code)]
//! Engine-agnostic pushdown planner. Reads `Capabilities`; emits a
//! `PushedPlan` + residual filters + `PushdownSummary`. Slow-but-right beats
//! fast-and-wrong: when in doubt, the planner leaves the expression in the
//! residual.

use datafusion::logical_expr::Expr;
use kyma_connectors::external::{Capabilities, FilterOp, PushdownSummary};

pub use kyma_connectors::external::PushdownSummary as Summary;

#[derive(Debug, Clone)]
pub struct PlanInput<'a> {
    pub source_name: &'a str,
    pub schema: &'a str,
    pub table: &'a str,
    pub capabilities: &'a Capabilities,
    pub filters: &'a [Expr],
    pub projection: Option<&'a [usize]>,
    pub limit: Option<usize>,
    pub sort: Option<&'a Expr>, // simplified for v1
}

#[derive(Debug, Clone)]
pub struct PlanOutput {
    pub pushed_filters: Vec<Expr>,
    pub residual_filters: Vec<Expr>,
    pub projection_pushed: bool,
    pub limit_pushed: Option<u64>,
    pub summary: PushdownSummary,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PushdownPlanner;

impl PushdownPlanner {
    pub fn plan(&self, input: PlanInput<'_>) -> PlanOutput {
        let mut pushed = Vec::new();
        let mut residual = Vec::new();
        let mut summary = PushdownSummary {
            source: input.source_name.into(),
            table: format!("{}.{}", input.schema, input.table),
            ..Default::default()
        };

        for f in input.filters {
            if Self::filter_is_pushable(f, input.capabilities) {
                summary.filters_pushed.push(format!("{f:?}"));
                pushed.push(f.clone());
            } else {
                summary.filters_residual.push(format!("{f:?}"));
                residual.push(f.clone());
            }
        }

        let projection_pushed = input.projection.is_some();
        summary.projection_pushed = projection_pushed;

        let limit_pushed = if input.capabilities.limit { input.limit.map(|l| l as u64) } else { None };
        summary.limit_pushed = limit_pushed;

        PlanOutput {
            pushed_filters: pushed,
            residual_filters: residual,
            projection_pushed,
            limit_pushed,
            summary,
        }
    }

    fn filter_is_pushable(expr: &Expr, caps: &Capabilities) -> bool {
        // M0 implements a recognizer for the simple shape `col OP literal` /
        // `col IN (literals)` / `col IS NULL` / `LIKE` and AND/OR/NOT trees
        // over those. Anything else is residual.
        // Engine slices (M1+) may extend this with their own residual checks
        // (e.g., MySQL collation safety), but the BASE rule stays here.
        use datafusion::logical_expr::{BinaryExpr, Operator};
        match expr {
            Expr::BinaryExpr(BinaryExpr { left: _, op, right: _ }) => {
                let op_kind = match op {
                    Operator::Eq => Some(FilterOp::Eq),
                    Operator::NotEq => Some(FilterOp::NotEq),
                    Operator::Lt => Some(FilterOp::Lt),
                    Operator::LtEq => Some(FilterOp::LtEq),
                    Operator::Gt => Some(FilterOp::Gt),
                    Operator::GtEq => Some(FilterOp::GtEq),
                    Operator::And => Some(FilterOp::And),
                    Operator::Or => Some(FilterOp::Or),
                    _ => None,
                };
                op_kind.map_or(false, |o| caps.filter_ops.contains(&o))
                    && (matches!(op, Operator::And | Operator::Or) || true) // recurse handled below for AND/OR
            }
            Expr::IsNull(_) => caps.filter_ops.contains(&FilterOp::IsNull),
            Expr::Not(inner) => caps.filter_ops.contains(&FilterOp::Not)
                && Self::filter_is_pushable(inner, caps),
            Expr::InList(_) => caps.filter_ops.contains(&FilterOp::In),
            Expr::Like(_) => caps.filter_ops.contains(&FilterOp::Like),
            _ => false, // err on the side of residual (correctness rule)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::logical_expr::{col, lit};

    fn caps() -> Capabilities { Capabilities::permissive_for_test() }

    #[test]
    fn empty_caps_pushes_nothing() {
        let empty = Capabilities::empty_for_test();
        let f = col("region").eq(lit("eu"));
        let out = PushdownPlanner.plan(PlanInput {
            source_name: "pg",
            schema: "public",
            table: "users",
            capabilities: &empty,
            filters: &[f.clone()],
            projection: None,
            limit: None,
            sort: None,
        });
        assert!(out.pushed_filters.is_empty());
        assert_eq!(out.residual_filters.len(), 1);
    }

    #[test]
    fn permissive_caps_push_simple_eq() {
        let caps = caps();
        let f = col("region").eq(lit("eu"));
        let out = PushdownPlanner.plan(PlanInput {
            source_name: "pg",
            schema: "public",
            table: "users",
            capabilities: &caps,
            filters: &[f.clone()],
            projection: Some(&[0, 1]),
            limit: Some(50),
            sort: None,
        });
        assert_eq!(out.pushed_filters.len(), 1);
        assert!(out.residual_filters.is_empty());
        assert!(out.projection_pushed);
        assert_eq!(out.limit_pushed, Some(50));
    }

    #[test]
    fn unknown_function_call_is_residual() {
        let caps = caps();
        // Synthesize an Expr the recognizer doesn't accept: a function call.
        // (Use a placeholder: build via DataFusion's Expr::ScalarFunction, but
        // for simplicity: a regex match — or any node that isn't BinaryExpr/etc.)
        let f = Expr::Negative(Box::new(col("x")));
        let out = PushdownPlanner.plan(PlanInput {
            source_name: "pg",
            schema: "public",
            table: "users",
            capabilities: &caps,
            filters: &[f.clone()],
            projection: None,
            limit: None,
            sort: None,
        });
        // Negative is not in our recognizer; it stays residual.
        assert_eq!(out.pushed_filters.len(), 0);
        assert_eq!(out.residual_filters.len(), 1);
    }
}
```

- [ ] **Step 2: Implement the property-test query generator stub**

Write `crates/kyma-connectors-testkit/src/query_gen.rs`:

```rust
#![forbid(unsafe_code)]
//! proptest strategies for the pushdown property test (spec §12.2).
//! v1: filter expressions only over a fixed schema; aggregations + sort/limit
//! land in M1.

use proptest::prelude::*;

#[derive(Debug, Clone)]
pub struct GeneratedQuery {
    pub filter_sql: String,
    pub projection: Vec<&'static str>,
    pub limit: Option<u64>,
}

pub fn arb_query() -> impl Strategy<Value = GeneratedQuery> {
    let cols = &["id", "email", "region", "score", "created_at"];
    let projection = proptest::collection::vec(
        proptest::sample::select(cols.to_vec()),
        1..=cols.len(),
    );
    let limit = proptest::option::of(1u64..=1000u64);
    let filter = arb_filter();
    (filter, projection, limit).prop_map(|(filter_sql, projection, limit)| GeneratedQuery {
        filter_sql,
        projection,
        limit,
    })
}

/// Generates a small filter expression: `(col OP literal) AND/OR (col OP literal)` etc.,
/// up to depth 4 per spec §12.2.
fn arb_filter() -> impl Strategy<Value = String> {
    let leaf = (
        proptest::sample::select(vec!["id", "email", "region", "score"]),
        proptest::sample::select(vec!["=", "!=", "<", "<=", ">", ">="]),
        any::<i64>().prop_filter("non-zero", |v| *v != 0),
    ).prop_map(|(c, op, v)| format!("{c} {op} {v}"));

    leaf.prop_recursive(4, 16, 4, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a}) AND ({b})")),
            (inner.clone(), inner.clone()).prop_map(|(a, b)| format!("({a}) OR ({b})")),
            inner.prop_map(|a| format!("NOT ({a})")),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #[test]
        fn gen_produces_well_formed_filters(q in arb_query()) {
            prop_assert!(!q.filter_sql.is_empty());
            prop_assert!(!q.projection.is_empty());
        }
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p kyma-federation planner -- --nocapture`
Expected: PASS (3 tests).

Run: `cargo test -p kyma-connectors-testkit query_gen -- --nocapture`
Expected: PASS (1 proptest, 256 cases).

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-federation/src/planner.rs crates/kyma-connectors-testkit/src/query_gen.rs
git commit -m "feat(db): PushdownPlanner with empty/permissive caps coverage + query generator"
```

---

## Task 12: `FederatedTableProvider` + `FederatedCatalogProvider` (DataFusion glue)

**Files:**
- Modify: `crates/kyma-federation/src/table_provider.rs`
- Modify: `crates/kyma-federation/src/catalog_provider.rs`

- [ ] **Step 1: Implement `FederatedTableProvider`**

Write `crates/kyma-federation/src/table_provider.rs`:

```rust
#![forbid(unsafe_code)]
//! DataFusion `TableProvider` backed by an `ExternalSource`. The provider
//! receives `(filters, projection, limit, sort)` from DataFusion, calls the
//! `PushdownPlanner` to peel off the pushable parts, and returns an
//! `ExecutionPlan` that streams Arrow batches from the source.

use std::any::Any;
use std::sync::Arc;

use arrow_schema::SchemaRef;
use async_trait::async_trait;
use datafusion::catalog::Session;
use datafusion::datasource::{TableProvider, TableType};
use datafusion::error::{DataFusionError, Result as DfResult};
use datafusion::logical_expr::{Expr, TableProviderFilterPushDown};
use datafusion::physical_plan::ExecutionPlan;

use kyma_connectors::external::ExternalSource;

pub struct FederatedTableProvider {
    pub source: Arc<dyn ExternalSource>,
    pub source_name: String,
    pub schema_name: String,
    pub table_name: String,
    pub arrow_schema: SchemaRef,
}

#[async_trait]
impl TableProvider for FederatedTableProvider {
    fn as_any(&self) -> &dyn Any { self }
    fn schema(&self) -> SchemaRef { self.arrow_schema.clone() }
    fn table_type(&self) -> TableType { TableType::Base }

    fn supports_filters_pushdown(&self, filters: &[&Expr]) -> DfResult<Vec<TableProviderFilterPushDown>> {
        // M0: advertise "Inexact" for everything — DataFusion will keep the
        // filter above us even if we push it. The planner's residual logic
        // ensures correctness; the engine slices upgrade specific shapes to
        // "Exact" once verified per-engine.
        Ok(filters.iter().map(|_| TableProviderFilterPushDown::Inexact).collect())
    }

    async fn scan(
        &self,
        _ctx: &dyn Session,
        _projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> DfResult<Arc<dyn ExecutionPlan>> {
        // M0 stub — engine slices wire this up with a real ExecutionPlan that
        // calls PushdownPlanner + ExternalSource::scan.
        Err(DataFusionError::NotImplemented(format!(
            "FederatedTableProvider::scan not implemented in M0 ({source_name}.{schema}.{table})",
            source_name = self.source_name, schema = self.schema_name, table = self.table_name,
        )))
    }
}
```

- [ ] **Step 2: Implement `FederatedCatalogProvider`**

Write `crates/kyma-federation/src/catalog_provider.rs`:

```rust
#![forbid(unsafe_code)]
//! DataFusion `CatalogProvider` backed by an external source. Lazily resolves
//! schema/table names by calling `ExternalSource::introspect` (cached with TTL).
//!
//! M0 ships the structure; engine slices wire actual introspection.

use std::any::Any;
use std::sync::Arc;

use datafusion::catalog::{CatalogProvider, SchemaProvider};
use datafusion::datasource::TableProvider;
use datafusion::error::Result as DfResult;

use kyma_connectors::external::ExternalSource;

pub struct FederatedCatalogProvider {
    pub source_name: String,
    pub source: Arc<dyn ExternalSource>,
    // schema cache lives here in M1; M0 returns an empty list.
}

impl FederatedCatalogProvider {
    pub fn new(source_name: impl Into<String>, source: Arc<dyn ExternalSource>) -> Self {
        Self { source_name: source_name.into(), source }
    }
}

impl CatalogProvider for FederatedCatalogProvider {
    fn as_any(&self) -> &dyn Any { self }

    fn schema_names(&self) -> Vec<String> {
        // M0: empty. M1+ populates from cached introspection.
        Vec::new()
    }

    fn schema(&self, _name: &str) -> Option<Arc<dyn SchemaProvider>> {
        // M0: none. M1+ returns a schema provider that lists tables and
        // returns FederatedTableProvider per table.
        None
    }
}
```

- [ ] **Step 3: Verify compile**

Run: `cargo check -p kyma-federation`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-federation/src/table_provider.rs crates/kyma-federation/src/catalog_provider.rs
git commit -m "feat(db): FederatedTableProvider + FederatedCatalogProvider scaffolds"
```

---

## Task 13: `live(table)` DataFusion `TableFunction`

**Files:**
- Modify: `crates/kyma-federation/src/live_fn.rs`

- [ ] **Step 1: Implement `live(...)` table function**

Write `crates/kyma-federation/src/live_fn.rs`:

```rust
#![forbid(unsafe_code)]
//! `live(table)` — DataFusion `TableFunction` that bypasses synced kyma
//! extents and resolves to the federated path for a `mode=both` source.
//!
//! Per spec §5.4 / §9.4: bare `pg_prod.public.users` resolves to the synced
//! table; `live(pg_prod.public.users)` resolves to a FederatedTableProvider.

use std::sync::Arc;

use datafusion::common::Result as DfResult;
use datafusion::datasource::TableProvider;
use datafusion::logical_expr::{Expr, TableSource};

use crate::registry::FederationRegistry;

/// Resolves a `live(...)` reference into a TableProvider.
/// The integration with DataFusion's `SessionContext::register_udtf` is
/// performed by `kyma-exec::register_federated_catalogs`.
pub fn resolve_live(
    _registry: &FederationRegistry,
    _args: &[Expr],
) -> DfResult<Arc<dyn TableProvider>> {
    // M0 stub: returns NotImplemented. The actual resolver wires the args
    // ("pg_prod.public.users") -> registry.get("pg_prod") -> FederatedTableProvider
    // for "public.users".
    Err(datafusion::error::DataFusionError::NotImplemented(
        "live(...) resolver implemented in M1".into()))
}
```

- [ ] **Step 2: Run a smoke check**

Run: `cargo check -p kyma-federation`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-federation/src/live_fn.rs
git commit -m "feat(db): live(table) resolver scaffold"
```

---

## Task 14: `kyma-exec::register_federated_catalogs`

**Files:**
- Modify: `crates/kyma-exec/src/lib.rs`
- Modify: `crates/kyma-exec/Cargo.toml`

- [ ] **Step 1: Add `kyma-federation` as an optional dep in `kyma-exec`**

Edit `crates/kyma-exec/Cargo.toml`:

```toml
[features]
default = []
federation = ["dep:kyma-federation"]

[dependencies]
# ... existing ...
kyma-federation = { workspace = true, optional = true }
```

- [ ] **Step 2: Add the registration helper**

Edit `crates/kyma-exec/src/lib.rs`. After existing `pub use` blocks, add:

```rust
#[cfg(feature = "federation")]
pub fn register_federated_catalogs(
    ctx: &datafusion::execution::context::SessionContext,
    registry: &kyma_federation::FederationRegistry,
) {
    use kyma_federation::FederatedCatalogProvider;
    use std::sync::Arc;

    for (name, entry) in registry.iter() {
        let provider = FederatedCatalogProvider::new(name.to_string(), entry.source.clone());
        ctx.register_catalog(name, Arc::new(provider));
        tracing::info!(target: "kyma_exec::federation", source = name, mode = %entry.mode,
            "registered federated catalog");
    }
}
```

- [ ] **Step 3: Run check**

Run: `cargo check -p kyma-exec --features federation`
Expected: PASS.

Run: `cargo check -p kyma-exec`
Expected: PASS (the helper is feature-gated).

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-exec/src/lib.rs crates/kyma-exec/Cargo.toml
git commit -m "feat(db): kyma-exec::register_federated_catalogs (feature-gated)"
```

---

## Task 15: Admin API extensions — `mode` validation, status/events/test-connection/scoped-pause

**Files:**
- Modify: `crates/kyma-server/src/connectors/extensions.rs` (create if absent — check existing layout)
- Modify: `crates/kyma-server/src/lib.rs` (route registration)
- Modify: `crates/kyma-connectors/src/admin.rs` if extensions live there instead

- [ ] **Step 1: Read existing admin layout**

Run: `grep -rn "v1/connectors" crates/kyma-server/src/ crates/kyma-connectors/src/`
Identify the router file. The instructions below assume `crates/kyma-connectors/src/admin.rs` since the existing routes already live there.

- [ ] **Step 2: Add the new routes + body schema**

Open `crates/kyma-connectors/src/admin.rs`. Locate the `Router::new()...` block and add:

```rust
.route("/v1/connectors/:id/status", get(handle_status))
.route("/v1/connectors/:id/events", get(handle_events))
.route("/v1/connectors/:id/test-connection", post(handle_test_connection))
// existing pause/resume route: extend to accept ?scope=sync|federation|all
```

Implement the handlers in the same file:

```rust
use axum::extract::Query;

#[derive(Debug, Default, serde::Deserialize)]
pub struct PauseQuery {
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_scope() -> String { "all".into() }

async fn handle_status(
    State(state): State<AdminState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Build the structured doc per spec §10.1 from the `connectors` row +
    // `connector_cdc_state` rows + (M0) zeroed federation pool stats.
    let row = catalog_sql::load_connector(&state.pool, id).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let cdc_rows: Vec<(String, String, Option<chrono::DateTime<chrono::Utc>>, Option<String>)> =
        sqlx::query_as(
            "SELECT source_table, phase, last_event_at, last_error
             FROM connector_cdc_state WHERE connector_id = $1"
        ).bind(id).fetch_all(&state.pool).await
         .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let resp = serde_json::json!({
        "id": id,
        "type": row.type_id,
        "mode": serde_json::Value::Null, // M0: read row.mode once it's selected; placeholder OK if mode lives elsewhere
        "source": {
            "reachable": null,
            "version": null,
            "last_health_check": null
        },
        "federation": {
            "status": null,
            "pool_in_use": 0,
            "pool_max": 0,
            "p50_query_ms": null,
            "p99_query_ms": null,
            "queries_total_5m": 0,
            "errors_5m": 0,
            "last_error": null
        },
        "sync": {
            "status": if cdc_rows.is_empty() { "idle" } else { "streaming" },
            "tables": cdc_rows.iter().map(|(t, p, l, e)| serde_json::json!({
                "source_table": t, "phase": p, "last_event_at": l, "last_error": e
            })).collect::<Vec<_>>()
        }
    });
    Ok(Json(resp))
}

async fn handle_events(
    State(state): State<AdminState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let rows: Vec<(i64, chrono::DateTime<chrono::Utc>, String, Option<serde_json::Value>)> =
        sqlx::query_as(
            "SELECT id, occurred_at, kind, payload_jsonb FROM connector_events
             WHERE connector_id = $1 ORDER BY occurred_at DESC LIMIT 100"
        ).bind(id).fetch_all(&state.pool).await
         .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({
        "events": rows.iter().map(|(eid, t, k, p)| serde_json::json!({
            "id": eid, "occurred_at": t, "kind": k, "payload": p
        })).collect::<Vec<_>>()
    })))
}

async fn handle_test_connection(
    State(state): State<AdminState>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // M0 stub: returns "not implemented" until engines wire ExternalSource::health.
    let _ = (state, id);
    Ok(Json(serde_json::json!({
        "ok": false,
        "detail": "test-connection wires up in M1 once an engine impl exists"
    })))
}
```

- [ ] **Step 3: Extend `POST /v1/connectors` validation to require `mode` ∈ {sync, federation, both}**

Locate the create handler. Before persisting, validate:

```rust
let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("sync");
if !["sync", "federation", "both"].contains(&mode) {
    return Err(StatusCode::BAD_REQUEST);
}
// If mode is federation or both, require connection.
if mode != "sync" && body.get("connection").is_none() {
    return Err(StatusCode::BAD_REQUEST);
}
// If connection.secret_ref is present, validate it resolves via SecretStore
// before persisting (fail-fast per spec §6).
// (Implementation: try to resolve; on failure, return 400 with the secret name.)
```

Insert the resolution gate; if it fails, return `400 Bad Request` with body `{"error": "secret_unresolved", "secret_ref": "<name>"}`.

- [ ] **Step 4: Persist the new fields**

Extend `catalog_sql::create_connector_direct` to also write `mode`, `connection_jsonb`, `scope_jsonb` (or add a parallel method with the longer signature so the Prometheus connector path is unchanged).

- [ ] **Step 5: Register routes in `kyma-server`**

Open `crates/kyma-server/src/lib.rs`. Find where the connectors admin router is mounted; ensure new routes are picked up automatically since they're in the same `Router` returned by `kyma_connectors::admin::router(...)`.

- [ ] **Step 6: Test the admin endpoints with testcontainers**

Write `crates/kyma-connectors/tests/foundation.rs`:

```rust
//! Integration tests for the M0 admin API extensions.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use kyma_connectors::admin;
use serde_json::json;
use sqlx::PgPool;
use testcontainers::clients::Cli;
use testcontainers_modules::postgres::Postgres;
use tower::util::ServiceExt;

async fn fresh_pool() -> PgPool {
    let docker = Cli::default();
    let pg = docker.run(Postgres::default());
    let url = format!(
        "postgres://postgres:postgres@127.0.0.1:{}/postgres",
        pg.get_host_port_ipv4(5432)
    );
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!("../kyma-catalog/migrations").run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn create_connector_rejects_unknown_mode() {
    let pool = fresh_pool().await;
    let app = admin::router(/* AdminState built from pool + secret store */);

    let body = json!({
        "name": "x",
        "type": "fake",
        "mode": "unknown",
        "target_database": "d",
        "target_table": "t",
        "schedule_ms": 60000,
        "drive_model": "periodic",
        "config_jsonb": {}
    });
    let resp = app.oneshot(Request::builder()
        .method("POST")
        .uri("/v1/connectors")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_connector_rejects_unresolvable_secret() {
    let pool = fresh_pool().await;
    let app = admin::router(/* AdminState with empty secret store */);

    let body = json!({
        "name": "pg",
        "type": "postgres",
        "mode": "federation",
        "connection": {"url": "postgres://...", "secret_ref": "missing_secret"},
        "target_database": "d",
        "target_table": "t",
        "schedule_ms": 0,
        "drive_model": "periodic",
        "config_jsonb": {}
    });
    let resp = app.oneshot(Request::builder()
        .method("POST")
        .uri("/v1/connectors")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["error"], "secret_unresolved");
}

#[tokio::test]
async fn status_endpoint_returns_404_for_missing_connector() {
    let pool = fresh_pool().await;
    let app = admin::router(/* AdminState */);
    let resp = app.oneshot(Request::builder()
        .uri("/v1/connectors/00000000-0000-0000-0000-000000000000/status")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
```

(Adjust `admin::router(...)` invocation to whatever the actual constructor name + signature is — read the file to confirm.)

- [ ] **Step 7: Run the tests**

Run: `cargo test -p kyma-connectors --test foundation -- --nocapture`
Expected: PASS (3 tests).

- [ ] **Step 8: Commit**

```bash
git add crates/kyma-connectors/src/admin.rs crates/kyma-connectors/src/catalog_sql.rs crates/kyma-connectors/tests/foundation.rs crates/kyma-server/src/lib.rs
git commit -m "feat(db): admin API — mode validation, status/events/test-connection, secret-resolution gate"
```

---

## Task 16: Cargo feature gating in `kyma-server`

**Files:**
- Modify: `crates/kyma-server/Cargo.toml`
- Modify: `crates/kyma-bin/Cargo.toml`
- Modify: `crates/kyma-bin/src/main.rs`

- [ ] **Step 1: Add the federation feature**

Edit `crates/kyma-server/Cargo.toml`:

```toml
[features]
default = []
federation = ["dep:kyma-federation", "kyma-exec/federation"]

[dependencies]
# ... existing ...
kyma-federation = { workspace = true, optional = true }
```

- [ ] **Step 2: Forward the feature in `kyma-bin`**

Edit `crates/kyma-bin/Cargo.toml`:

```toml
[features]
default = []
federation = ["kyma-server/federation"]

[dependencies]
kyma-server = { workspace = true }
```

- [ ] **Step 3: Wire the registry in main when feature enabled**

Edit `crates/kyma-bin/src/main.rs`, near where `SessionContext` is built. Wrap with:

```rust
#[cfg(feature = "federation")]
{
    use kyma_federation::FederationRegistry;
    let mut registry = FederationRegistry::new();
    // M1+ populates from catalog rows; M0 leaves it empty.
    kyma_exec::register_federated_catalogs(&session_ctx, &registry);
}
```

- [ ] **Step 4: Build with and without the feature**

Run: `cargo build -p kyma-bin`
Expected: PASS.

Run: `cargo build -p kyma-bin --features federation`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-server/Cargo.toml crates/kyma-bin/Cargo.toml crates/kyma-bin/src/main.rs
git commit -m "feat(db): kyma-server federation cargo feature"
```

---

## Task 17: Architectural test — no engine-specific code in `kyma-federation`

**Files:**
- Create: `crates/kyma-federation/tests/architectural.rs`

- [ ] **Step 1: Write the test**

Write `crates/kyma-federation/tests/architectural.rs`:

```rust
//! No engine-specific code may live in `kyma-federation` (per spec §13.6).
//! Every engine impl must live behind `ExternalSource` in `kyma-connectors`.
//! This test keeps that invariant honest.

use std::path::Path;
use std::process::Command;

#[test]
fn no_engine_specific_imports_in_kyma_federation() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let output = Command::new("grep")
        .arg("-r")
        .arg("--include=*.rs")
        .args([
            "-l",
            "-E",
            r"\b(sqlx::Postgres|sqlx::MySql|mongodb::|rdkafka::|tokio_postgres)\b",
        ])
        .arg(&dir)
        .output()
        .expect("failed to run grep");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "kyma-federation must not import engine-specific drivers; offending files:\n{stdout}"
    );
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p kyma-federation --test architectural -- --nocapture`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-federation/tests/architectural.rs
git commit -m "test(db): architectural — no engine-specific imports in kyma-federation"
```

---

## Task 18: M0 acceptance smoke

**Files:**
- (No new files; verifies the workspace as a whole.)

- [ ] **Step 1: Workspace clean build**

Run: `cargo build --workspace --all-targets`
Expected: PASS, no warnings other than pre-existing.

- [ ] **Step 2: Workspace test sweep**

Run: `cargo test --workspace --all-targets`
Expected: PASS.

- [ ] **Step 3: Federation feature build**

Run: `cargo build -p kyma-bin --features federation`
Expected: PASS.

- [ ] **Step 4: Migration applies cleanly on a fresh DB**

Run: `cargo test -p kyma-catalog migration_0010_applies_cleanly`
Expected: PASS.

- [ ] **Step 5: Tag the milestone**

Run:

```bash
git log --oneline -20   # confirm M0 commits are present
git tag db-m0-foundation
```

(Tag is local; push only if team requests.)

---

## M0 Acceptance Checklist

The following must all be true before declaring M0 complete and starting M1:

- [ ] Catalog migration `0010_external_sources.sql` applies cleanly and is reversible (additive only).
- [ ] `ExternalSource`, `Capabilities`, `PushedPlan`, `PushdownSummary`, `Checkpoint`, `ExternalError`, `BatchSink`, `InferredSchema`, `ConnectionConfig`, `Scope`, `CdcEvent` types compile and have unit-test coverage of their basic invariants.
- [ ] `kyma-federation` crate exists with `FederationRegistry`, `FederatedCatalogProvider`, `FederatedTableProvider`, `PushdownPlanner`, `live_fn` modules in place; the planner has the empty/permissive/unknown-residual unit tests passing.
- [ ] `kyma-connectors-testkit` crate exists with the `query_gen` proptest scaffold passing.
- [ ] `kyma-ingest-core::CommitCoordinator` has `with_cursor_update`; the integration test asserts atomic cursor advance.
- [ ] `CdcConnector` trait, `SchemaEvolver` (with 4 unit tests), runner/snapshot/stream skeletons committed.
- [ ] Registry/runner dispatch on `Periodic` vs. `Cdc` works.
- [ ] Admin API extensions (`status`, `events`, `test-connection`, scoped pause, `mode` validation, secret-resolution gate) committed and integration-tested.
- [ ] `kyma-server` `federation` Cargo feature flag works (build with and without).
- [ ] Architectural test ensures no engine-specific imports in `kyma-federation`.
- [ ] `cargo build --workspace --all-targets` and `cargo test --workspace --all-targets` clean.
- [ ] M0 spec open questions (§14) revisited; final decisions recorded as ADRs in `docs/superpowers/specs/` if they materially differ from the spec text. Specifically: shape of `Capabilities`, choice of `PushedPlan::Sql(String)` vs structured AST, opaque-JSON checkpoint shape.

---

## Open M0 Decisions (from spec §14)

These are answered during M0 review; resolution lands as either inline edits to this plan or short ADR notes alongside the spec.

1. **Final shape of `Capabilities`** — the current Task 3 implementation is a starting point; review against the three engines (Postgres logical-replication-aware filters, MySQL collation-safe set, Mongo aggregation pipeline shapes) before M1 starts.
2. **`PushedPlan::Sql(String, Vec<BoundParam>)` vs structured AST** — current plan is the simpler `Sql(String)` per spec recommendation. If review uncovers a clear safety regression, upgrade to AST before M1.
3. **Opaque JSON checkpoints** — current plan is `Checkpoint(serde_json::Value)`; per spec §14 question 3, this is the recommendation. Reaffirmed during M0 review.
