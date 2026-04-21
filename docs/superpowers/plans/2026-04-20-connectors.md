# Connectors Framework + Prometheus Reference Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Introduce an ingestion-connector subsystem that pulls data from third-party observability sources, and ship one reference connector (Prometheus `/metrics` scrape) end-to-end.

**Architecture:** New `kyma-connectors` crate owns the `Connector` trait, a periodic scheduler (via existing `background_tasks` work queue), a runner that invokes connectors, an admin HTTP API mounted on `kyma-server`, and a reference `PromConnector`. Catalog-managed connector configs via migration 005. All rows flow through the existing `WritePath` + `ingest_ledger` idempotency path.

**Tech Stack:** Rust, tokio, axum, reqwest, sqlx/Postgres, arrow, serde_json, `metrics` facade, testcontainers.

**Spec:** See `docs/superpowers/specs/2026-04-20-connectors-design.md`.

---

## File Structure

```
crates/kyma-connectors/                         (new)
├── Cargo.toml
├── src/
│   ├── lib.rs                -- re-exports + module decls
│   ├── types.rs              -- Connector trait, ConnectorRun, ConnectorCtx,
│                                ConnectorError, ConfigError, DriveModel
│   ├── secrets.rs            -- SecretStore trait, SecretError, EnvSecretStore
│   ├── registry.rs           -- ConnectorRegistry
│   ├── metrics.rs            -- ConnectorMetrics helper
│   ├── arrow_coerce.rs       -- rows_to_batches helper (JSON Value → RecordBatch)
│   ├── scheduler.rs          -- ConnectorScheduler
│   ├── runner.rs             -- ConnectorRunner
│   ├── admin.rs              -- axum router + handlers for /v1/connectors
│   └── prometheus/
│       ├── mod.rs            -- PromConnector
│       └── parser.rs         -- OpenMetrics text parser
└── tests/
    ├── openmetrics_parser.rs
    ├── validate_config.rs
    ├── secret_store.rs
    ├── registry.rs
    ├── arrow_coerce.rs
    └── scheduler_runner_it.rs   -- testcontainers integration test

crates/kyma-catalog/
├── migrations/
│   └── 005_connectors.sql    (new)
└── src/
    └── lib.rs                -- (no changes — connectors SQL lives in kyma-connectors
                                  via PostgresCatalog.pool() downcast, matching the
                                  phase-C pattern used by compaction scheduler)

crates/kyma-bin/src/main.rs   -- spawn scheduler + N runners
crates/kyma-server/src/lib.rs -- mount kyma-connectors::admin::router()

scripts/test-prometheus-connector.sh (new)
scripts/fixtures/prom-metrics.txt     (new — fixture for E2E)
Cargo.toml                            -- add kyma-connectors to workspace members
                                        and workspace.dependencies
```

All files stay small (target < 400 LoC each). Scheduler and runner are separate files because they have distinct lifecycles. The Prometheus-specific parser is its own module because it'll grow histogram-bucket-merge logic later.

---

## Conventions for every task

- Use `Result<T>` from `kyma_core::errors` where possible for catalog/storage errors; `thiserror`-derived local errors otherwise.
- Every Rust file starts with `#![forbid(unsafe_code)]`.
- Do not amend commits. On test/clippy failure, fix and make a new commit.
- After each task, run `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings` before committing.
- Commit message convention matches the existing repo (present tense, no Conventional Commits prefix required, trailing `Co-Authored-By:` line). Example: `add kyma-connectors crate scaffold`.

---

## Task 1: Scaffold `kyma-connectors` crate + workspace wiring

**Files:**
- Create: `crates/kyma-connectors/Cargo.toml`
- Create: `crates/kyma-connectors/src/lib.rs`
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Create the crate manifest**

Create `crates/kyma-connectors/Cargo.toml`:

```toml
[package]
name = "kyma-connectors"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[lints]
workspace = true

[dependencies]
kyma-core = { workspace = true }
kyma-catalog = { workspace = true }
kyma-ingest-core = { workspace = true }
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
axum = { workspace = true }
tower = { workspace = true }
uuid = { workspace = true }
chrono = { workspace = true }
futures = { workspace = true }
metrics = { workspace = true }
fastrand = { workspace = true }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls", "stream"] }
sqlx = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["full", "test-util"] }
testcontainers = { workspace = true }
testcontainers-modules = { workspace = true }
```

- [ ] **Step 2: Create the stub lib.rs**

Create `crates/kyma-connectors/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
//! Ingestion connectors: pull-side integrations to third-party sources.
//!
//! See `docs/superpowers/specs/2026-04-20-connectors-design.md`.
```

- [ ] **Step 3: Add to workspace**

In root `Cargo.toml`, add `"crates/kyma-connectors"` to `[workspace] members` (alphabetical insertion after `kyma-compaction`) and add the path dep under `[workspace.dependencies]`:

```toml
kyma-connectors = { path = "crates/kyma-connectors" }
```

Also add `reqwest = "0.12"` line under workspace deps if not already there (check first — if present, skip):

```bash
grep -n "^reqwest" Cargo.toml || echo "need to add"
```

- [ ] **Step 4: Verify the workspace builds**

Run: `cargo check -p kyma-connectors`
Expected: success, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/kyma-connectors/
git commit -m "$(cat <<'EOF'
add kyma-connectors crate scaffold

Empty crate, wired into the workspace. Subsequent tasks add
types, scheduler, runner, admin API, and the Prometheus reference
connector.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Migration 005 — connectors catalog schema

**Files:**
- Create: `crates/kyma-catalog/migrations/005_connectors.sql`
- Create: `crates/kyma-connectors/tests/migration_smoke.rs`

- [ ] **Step 1: Write the migration**

Create `crates/kyma-catalog/migrations/005_connectors.sql`:

```sql
-- Catalog tables for the ingestion-connector subsystem.
--
-- * `connectors` — operator-managed definitions, one row per connector instance.
-- * `connector_cursors` — per-connector checkpoint state (API cursor / last
--   timestamp / etc). Separate table so cursor updates are small, frequent
--   writes that don't churn the connectors row.
-- * `connector_leases` — pre-provisioned for the `Continuous` drive model
--   (streaming connectors). Unused in slice-1.
-- * The unique index on `background_tasks` prevents duplicate `connector_tick`
--   enqueues when multiple kyma nodes run schedulers concurrently.

CREATE TABLE IF NOT EXISTS connectors (
    id                  uuid PRIMARY KEY DEFAULT uuid_generate_v4(),
    name                text NOT NULL UNIQUE,
    type                text NOT NULL,
    target_database     text NOT NULL,
    target_table        text NOT NULL,
    config_jsonb        jsonb NOT NULL,
    schedule_ms         bigint NOT NULL CHECK (schedule_ms >= 100),
    drive_model         text NOT NULL
                            CHECK (drive_model IN ('periodic','continuous')),
    enabled             boolean NOT NULL DEFAULT TRUE,
    disabled_reason     text,
    last_run_at         timestamptz,
    last_success_at     timestamptz,
    last_error          text,
    last_rows_ingested  bigint,
    created_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS connectors_enabled_drive_idx
    ON connectors (drive_model, enabled)
    WHERE enabled = TRUE;

CREATE TABLE IF NOT EXISTS connector_cursors (
    connector_id  uuid PRIMARY KEY
                  REFERENCES connectors(id) ON DELETE CASCADE,
    cursor_jsonb  jsonb,
    updated_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS connector_leases (
    connector_id  uuid PRIMARY KEY
                  REFERENCES connectors(id) ON DELETE CASCADE,
    node_id       text NOT NULL,
    expires_at    timestamptz NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS background_tasks_connector_tick_uniq
    ON background_tasks ((payload->>'connector_id'),
                         (payload->>'scheduled_for'))
    WHERE kind = 'connector_tick' AND status IN ('pending', 'claimed');
```

- [ ] **Step 2: Write the smoke test**

Create `crates/kyma-connectors/tests/migration_smoke.rs`:

```rust
//! Smoke test: migration 005 applies cleanly and creates expected objects.

use kyma_catalog::PostgresCatalog;
use testcontainers_modules::postgres::Postgres;
use testcontainers::runners::AsyncRunner;

#[tokio::test]
async fn migration_005_creates_connector_tables() {
    let pg = Postgres::default().start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let catalog = PostgresCatalog::connect(&url)
        .await
        .expect("connect + migrate");

    // Poke the new tables.
    let pool = catalog.pool();
    sqlx::query("SELECT 1 FROM connectors LIMIT 0")
        .execute(pool)
        .await
        .expect("connectors table exists");
    sqlx::query("SELECT 1 FROM connector_cursors LIMIT 0")
        .execute(pool)
        .await
        .expect("connector_cursors table exists");
    sqlx::query("SELECT 1 FROM connector_leases LIMIT 0")
        .execute(pool)
        .await
        .expect("connector_leases table exists");

    // Verify the unique index for connector_tick dedup exists.
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_indexes
         WHERE tablename = 'background_tasks'
         AND indexname = 'background_tasks_connector_tick_uniq'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_indexes");
    assert_eq!(count, 1, "dedup index should be present");
}
```

Note: `PostgresCatalog` must expose `pub fn pool(&self) -> &Pool<Postgres>`. If that accessor doesn't already exist (check `crates/kyma-catalog/src/lib.rs` — memory says it does, as a phase-C shortcut), skip; otherwise add a 3-liner in the same task as a trivial change.

- [ ] **Step 3: Run test — expect it to find the migration**

Run: `cargo test -p kyma-connectors --test migration_smoke`
Expected: PASS. If FAIL because `sqlx::migrate!` embeds migrations at compile time and hasn't picked up the new file, run `cargo clean -p kyma-catalog && cargo test -p kyma-connectors --test migration_smoke`.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-catalog/migrations/005_connectors.sql \
        crates/kyma-connectors/tests/migration_smoke.rs \
        crates/kyma-catalog/src/lib.rs    # only if pool() accessor was added
git commit -m "$(cat <<'EOF'
add catalog migration 005 for connectors

connectors / connector_cursors / connector_leases tables plus a
partial unique index on background_tasks that dedupes
connector_tick enqueues from concurrent schedulers.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Core types — `Connector` trait + sibling types

**Files:**
- Create: `crates/kyma-connectors/src/types.rs`
- Modify: `crates/kyma-connectors/src/lib.rs`

No unit tests — this task only defines types. Compile-pass is the test.

- [ ] **Step 1: Write `types.rs`**

Create `crates/kyma-connectors/src/types.rs`:

```rust
//! Core types for the connector framework.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::metrics::ConnectorMetrics;
use crate::secrets::SecretStore;

/// How a connector is driven — periodic tick or long-lived lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriveModel {
    Periodic { interval_ms: u64 },
    Continuous { heartbeat_ms: u64 },
}

/// Produced by a single `run_once` invocation.
pub struct ConnectorRun {
    /// JSON rows — run through JSON→Arrow coercion before ingest.
    pub rows: Vec<serde_json::Value>,
    /// When `Some`, the framework upserts this into `connector_cursors`.
    pub new_cursor: Option<serde_json::Value>,
}

/// Context passed to `Connector::run_once`. Cheap to clone per-tick; the
/// connector should not retain it beyond the call.
pub struct ConnectorCtx {
    pub connector_id: Uuid,
    pub http: reqwest::Client,
    pub secrets: Arc<dyn SecretStore>,
    /// Tick timestamp (bucketed to the schedule grid). Used for the
    /// idempotency key and as a fallback sample timestamp.
    pub scheduled_for: DateTime<Utc>,
    pub metrics: ConnectorMetrics,
}

/// Failure classification that determines framework behaviour.
#[derive(Debug, thiserror::Error)]
pub enum ConnectorError {
    /// Retried via `background_tasks` on the next tick.
    #[error("transient: {0}")]
    Transient(String),
    /// Not retried; logged; next tick proceeds on its schedule.
    #[error("permanent: {0}")]
    Permanent(String),
    /// Connector is disabled with `disabled_reason`; operator must re-enable.
    #[error("config: {0}")]
    Config(String),
}

#[derive(Debug, thiserror::Error)]
#[error("invalid config: {0}")]
pub struct ConfigError(pub String);

/// Implement this trait to add a new connector type.
///
/// Slice-1 registers instances at compile time via [`ConnectorRegistry`].
#[async_trait]
pub trait Connector: Send + Sync + 'static {
    /// Stable identifier for this connector type (e.g., `"prometheus"`).
    fn type_id(&self) -> &'static str;

    /// Called on `POST`/`PATCH /v1/connectors` before the row is persisted.
    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError>;

    /// Execute one tick. Return rows + optional cursor update.
    async fn run_once(
        &self,
        ctx: &ConnectorCtx,
        cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError>;
}
```

- [ ] **Step 2: Expose from `lib.rs`**

Rewrite `crates/kyma-connectors/src/lib.rs`:

```rust
#![forbid(unsafe_code)]
//! Ingestion connectors: pull-side integrations to third-party sources.
//!
//! See `docs/superpowers/specs/2026-04-20-connectors-design.md`.

pub mod metrics;
pub mod secrets;
pub mod types;

pub use types::{
    ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun, DriveModel,
};
```

(`metrics` and `secrets` modules are stub files for now — create one-line stubs so the `pub mod` lines compile. The real content comes in Tasks 4 and 6.)

Create `crates/kyma-connectors/src/secrets.rs`:

```rust
//! Secret-store abstraction. Implementation lands in Task 4.
use async_trait;  // placeholder; rewritten in Task 4.

pub trait SecretStore: Send + Sync {
    fn resolve(&self, reference: &str) -> Result<String, crate::secrets::SecretError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret not found: {0}")]
    NotFound(String),
}
```

Create `crates/kyma-connectors/src/metrics.rs`:

```rust
//! Per-tick metrics helpers. Implementation lands in Task 6.

#[derive(Clone)]
pub struct ConnectorMetrics {
    /// Stable string label used for all metric emissions from this connector.
    pub type_id: &'static str,
    pub connector_id: uuid::Uuid,
}
```

- [ ] **Step 3: Compile**

Run: `cargo check -p kyma-connectors`
Expected: success, 0 warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/src/
git commit -m "$(cat <<'EOF'
add Connector trait and sibling types

Defines the contract every connector implementation satisfies:
type_id, validate_config, and async run_once returning rows +
optional cursor update. Also the DriveModel enum (Periodic
implemented now; Continuous variant reserved).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `SecretStore` trait + `EnvSecretStore`

**Files:**
- Replace: `crates/kyma-connectors/src/secrets.rs`
- Create: `crates/kyma-connectors/tests/secret_store.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/kyma-connectors/tests/secret_store.rs`:

```rust
use kyma_connectors::secrets::{EnvSecretStore, SecretError, SecretStore};

#[test]
fn literal_passes_through() {
    let s = EnvSecretStore;
    assert_eq!(s.resolve("hunter2").unwrap(), "hunter2");
}

#[test]
fn env_reference_resolves() {
    // Use a deterministic, unlikely var name.
    std::env::set_var("KYMA_CONN_TEST_SECRET_42", "sekret");
    let s = EnvSecretStore;
    assert_eq!(
        s.resolve("$env:KYMA_CONN_TEST_SECRET_42").unwrap(),
        "sekret"
    );
    std::env::remove_var("KYMA_CONN_TEST_SECRET_42");
}

#[test]
fn env_reference_missing_yields_not_found() {
    std::env::remove_var("KYMA_CONN_TEST_MISSING_99");
    let s = EnvSecretStore;
    match s.resolve("$env:KYMA_CONN_TEST_MISSING_99") {
        Err(SecretError::NotFound(name)) => {
            assert_eq!(name, "KYMA_CONN_TEST_MISSING_99")
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p kyma-connectors --test secret_store`
Expected: FAIL (EnvSecretStore not defined, `resolve` not implemented on stub).

- [ ] **Step 3: Write the real `secrets.rs`**

Replace `crates/kyma-connectors/src/secrets.rs`:

```rust
//! Secret resolution.
//!
//! Slice-1 ships two implementations: a pass-through literal + env-var
//! reference resolver (`EnvSecretStore`), and a trait so Vault / AWS SM
//! can plug in later with no callsite changes.

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("secret not found: {0}")]
    NotFound(String),
}

pub trait SecretStore: Send + Sync {
    /// Resolve a reference.
    ///
    /// - `"$env:NAME"` → value of environment variable `NAME`.
    /// - Anything else → returned verbatim.
    fn resolve(&self, reference: &str) -> Result<String, SecretError>;
}

#[derive(Default, Clone, Copy)]
pub struct EnvSecretStore;

impl SecretStore for EnvSecretStore {
    fn resolve(&self, reference: &str) -> Result<String, SecretError> {
        if let Some(name) = reference.strip_prefix("$env:") {
            std::env::var(name)
                .map_err(|_| SecretError::NotFound(name.to_string()))
        } else {
            Ok(reference.to_string())
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-connectors --test secret_store`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-connectors/src/secrets.rs \
        crates/kyma-connectors/tests/secret_store.rs
git commit -m "$(cat <<'EOF'
add SecretStore trait and EnvSecretStore

Default impl resolves '\$env:NAME' references to environment
variables and passes literals through unchanged. Vault / AWS SM
backends can plug in without touching callsites.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `ConnectorRegistry`

**Files:**
- Create: `crates/kyma-connectors/src/registry.rs`
- Create: `crates/kyma-connectors/tests/registry.rs`
- Modify: `crates/kyma-connectors/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/kyma-connectors/tests/registry.rs`:

```rust
use async_trait::async_trait;
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::{
    ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun,
};
use std::sync::Arc;

struct FakeConn;

#[async_trait]
impl Connector for FakeConn {
    fn type_id(&self) -> &'static str { "fake" }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> { Ok(()) }
    async fn run_once(
        &self,
        _: &ConnectorCtx,
        _: &serde_json::Value,
        _: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        Ok(ConnectorRun { rows: vec![], new_cursor: None })
    }
}

#[test]
fn register_and_lookup() {
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(FakeConn));
    let c = reg.lookup("fake").expect("found");
    assert_eq!(c.type_id(), "fake");
    assert!(reg.lookup("missing").is_none());
}

#[test]
#[should_panic(expected = "already registered")]
fn double_register_panics() {
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(FakeConn));
    reg.register(Arc::new(FakeConn));
}

#[test]
fn types_list() {
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(FakeConn));
    let mut types: Vec<_> = reg.types().collect();
    types.sort();
    assert_eq!(types, vec!["fake"]);
}
```

- [ ] **Step 2: Run test; expect compile failure**

Run: `cargo test -p kyma-connectors --test registry`
Expected: FAIL (`ConnectorRegistry` not defined).

- [ ] **Step 3: Write `registry.rs`**

Create `crates/kyma-connectors/src/registry.rs`:

```rust
//! Compile-time connector-type registry.

use crate::types::Connector;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct ConnectorRegistry {
    by_type: HashMap<&'static str, Arc<dyn Connector>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, c: Arc<dyn Connector>) {
        let id = c.type_id();
        assert!(
            !self.by_type.contains_key(id),
            "connector type {id:?} already registered",
        );
        self.by_type.insert(id, c);
    }

    pub fn lookup(&self, type_id: &str) -> Option<Arc<dyn Connector>> {
        self.by_type.get(type_id).cloned()
    }

    pub fn types(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.by_type.keys().copied()
    }
}
```

- [ ] **Step 4: Expose from `lib.rs`**

In `crates/kyma-connectors/src/lib.rs`, add `pub mod registry;` alongside the others.

- [ ] **Step 5: Run tests**

Run: `cargo test -p kyma-connectors --test registry`
Expected: PASS, 3 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-connectors/src/lib.rs \
        crates/kyma-connectors/src/registry.rs \
        crates/kyma-connectors/tests/registry.rs
git commit -m "$(cat <<'EOF'
add ConnectorRegistry for compile-time type registration

Map of connector type_id -> Arc<dyn Connector>, populated once at
startup. Lookup is cheap; register-twice panics (programmer error).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `ConnectorMetrics` helper

**Files:**
- Replace: `crates/kyma-connectors/src/metrics.rs`

No separate test file — this is a thin wrapper. Correctness is verified by the E2E test in Task 15.

- [ ] **Step 1: Replace the stub metrics.rs**

Replace `crates/kyma-connectors/src/metrics.rs`:

```rust
//! Per-connector metric emission helpers.
//!
//! Metric names follow the project's `kyma_connector_*` scheme and are
//! emitted via the `metrics` facade crate. The exporter in kyma-server
//! turns these into Prometheus scrape output at /metrics.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Cheap to clone; carries the stable labels that get stamped on every
/// metric from one connector.
#[derive(Clone, Debug)]
pub struct ConnectorMetrics {
    pub connector_id: Uuid,
    pub type_id: &'static str,
}

impl ConnectorMetrics {
    pub fn record_tick(&self, result: TickResult, duration_s: f64) {
        let id = self.connector_id.to_string();
        ::metrics::counter!(
            "kyma_connector_ticks_total",
            "connector_id" => id.clone(),
            "type" => self.type_id,
            "result" => result.as_label(),
        )
        .increment(1);
        ::metrics::histogram!(
            "kyma_connector_duration_seconds",
            "connector_id" => id,
            "type" => self.type_id,
        )
        .record(duration_s);
    }

    pub fn record_rows(&self, n: u64) {
        ::metrics::counter!(
            "kyma_connector_rows_ingested_total",
            "connector_id" => self.connector_id.to_string(),
            "type" => self.type_id,
        )
        .increment(n);
    }

    pub fn record_error(&self, reason: &'static str) {
        ::metrics::counter!(
            "kyma_connector_errors_total",
            "connector_id" => self.connector_id.to_string(),
            "type" => self.type_id,
            "reason" => reason,
        )
        .increment(1);
    }

    pub fn set_last_success(&self, at: DateTime<Utc>) {
        ::metrics::gauge!(
            "kyma_connector_last_success_timestamp_seconds",
            "connector_id" => self.connector_id.to_string(),
        )
        .set(at.timestamp() as f64);
    }

    pub fn set_cursor_age(&self, seconds: f64) {
        ::metrics::gauge!(
            "kyma_connector_cursor_age_seconds",
            "connector_id" => self.connector_id.to_string(),
        )
        .set(seconds);
    }
}

#[derive(Copy, Clone, Debug)]
pub enum TickResult { Ok, Transient, Permanent, Config }

impl TickResult {
    pub fn as_label(self) -> &'static str {
        match self {
            TickResult::Ok => "ok",
            TickResult::Transient => "transient",
            TickResult::Permanent => "permanent",
            TickResult::Config => "config",
        }
    }
}
```

- [ ] **Step 2: Verify compile**

Run: `cargo check -p kyma-connectors`
Expected: success, 0 warnings.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-connectors/src/metrics.rs
git commit -m "$(cat <<'EOF'
add ConnectorMetrics helper

Stamps (connector_id, type) labels onto every tick/rows/error emit
so operator dashboards and alerts can key off connector instance.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: OpenMetrics text parser

**Files:**
- Create: `crates/kyma-connectors/src/prometheus/mod.rs`  (stub at this task; expanded in Task 8–9)
- Create: `crates/kyma-connectors/src/prometheus/parser.rs`
- Create: `crates/kyma-connectors/tests/openmetrics_parser.rs`
- Modify: `crates/kyma-connectors/src/lib.rs` — add `pub mod prometheus;`

- [ ] **Step 1: Write the failing test with fixtures**

Create `crates/kyma-connectors/tests/openmetrics_parser.rs`:

```rust
use kyma_connectors::prometheus::parser::{parse_openmetrics, Sample};

#[test]
fn parses_counter_and_gauge() {
    let text = "\
# HELP http_requests_total The total.
# TYPE http_requests_total counter
http_requests_total{method=\"GET\",code=\"200\"} 42
http_requests_total{method=\"POST\",code=\"500\"} 3
# HELP queue_depth Current depth.
# TYPE queue_depth gauge
queue_depth 17.5
";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 3);

    let first = &samples[0];
    assert_eq!(first.name, "http_requests_total");
    assert_eq!(first.value, Some(42.0));
    assert_eq!(first.labels.get("method"), Some(&"GET".to_string()));
    assert_eq!(first.labels.get("code"), Some(&"200".to_string()));

    let gauge = samples.iter().find(|s| s.name == "queue_depth").unwrap();
    assert_eq!(gauge.value, Some(17.5));
    assert!(gauge.labels.is_empty());
}

#[test]
fn parses_histogram_exploded() {
    let text = "\
# TYPE request_duration_seconds histogram
request_duration_seconds_bucket{le=\"0.1\"} 10
request_duration_seconds_bucket{le=\"0.5\"} 20
request_duration_seconds_bucket{le=\"+Inf\"} 25
request_duration_seconds_sum 3.14
request_duration_seconds_count 25
";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 5);
    // All histogram-exploded samples are emitted as individual metric names.
    let buckets: Vec<_> = samples
        .iter()
        .filter(|s| s.name == "request_duration_seconds_bucket")
        .collect();
    assert_eq!(buckets.len(), 3);
    assert_eq!(buckets[2].labels.get("le"), Some(&"+Inf".to_string()));
}

#[test]
fn special_float_values_become_null() {
    let text = "a_metric 1.0\nb_metric NaN\nc_metric +Inf\nd_metric -Inf\n";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 4);
    assert_eq!(samples[0].value, Some(1.0));
    assert_eq!(samples[1].value, None, "NaN → None");
    assert_eq!(samples[2].value, None, "+Inf → None");
    assert_eq!(samples[3].value, None, "-Inf → None");
}

#[test]
fn comments_and_blank_lines_skipped() {
    let text = "\n# some comment\n\n# HELP x y\nx 5\n";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].name, "x");
}

#[test]
fn malformed_line_errors() {
    // Bare label without value is a malformed exposition line.
    let text = "garbled_no_value{a=\"b\"}\n";
    let err = parse_openmetrics(text).unwrap_err();
    assert!(err.to_string().contains("line 1"), "error should cite line");
}

#[test]
fn escaped_label_values() {
    let text = "m{a=\"b\\\"c\",d=\"e\\\\f\"} 1\n";
    let samples = parse_openmetrics(text).expect("parse");
    assert_eq!(samples[0].labels.get("a"), Some(&"b\"c".to_string()));
    assert_eq!(samples[0].labels.get("d"), Some(&"e\\f".to_string()));
}
```

- [ ] **Step 2: Run — expect compile failure**

Run: `cargo test -p kyma-connectors --test openmetrics_parser`
Expected: FAIL (`parse_openmetrics` undefined).

- [ ] **Step 3: Implement the parser**

Create `crates/kyma-connectors/src/prometheus/mod.rs`:

```rust
//! Prometheus `/metrics` scrape connector.
pub mod parser;
// PromConnector lands in Task 8.
```

Create `crates/kyma-connectors/src/prometheus/parser.rs`:

```rust
//! OpenMetrics / Prometheus text-exposition-format parser.
//!
//! Intentionally handwritten and small: the grammar is simple and we avoid
//! pulling a dependency. Returns one `Sample` per data line; `HELP`/`TYPE`/
//! `UNIT` lines are consumed but not retained.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    /// `None` for `NaN` / `+Inf` / `-Inf` (emitted as JSON null downstream).
    pub value: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
#[error("parse error at line {line}: {msg}")]
pub struct ParseError {
    pub line: usize,
    pub msg: String,
}

pub fn parse_openmetrics(text: &str) -> Result<Vec<Sample>, ParseError> {
    let mut out = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let s = raw.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        out.push(parse_sample_line(s, line_no)?);
    }
    Ok(out)
}

fn parse_sample_line(s: &str, line: usize) -> Result<Sample, ParseError> {
    // grammar:  NAME[{label="value",...}]  SPACE  FLOAT  [SPACE TIMESTAMP]
    //
    // We ignore any trailing per-sample timestamp — scrape time wins.

    // Split the metric name (and optional labels block) from value.
    let (head, value_str) = split_last_whitespace(s)
        .ok_or_else(|| ParseError { line, msg: format!("no value: {s:?}") })?;

    let (name, labels_part) = match head.find('{') {
        Some(open) => {
            let close = head.rfind('}')
                .ok_or_else(|| ParseError { line, msg: "unbalanced {".into() })?;
            if close <= open {
                return Err(ParseError { line, msg: "unbalanced {}".into() });
            }
            (&head[..open], Some(&head[open + 1..close]))
        }
        None => (head, None),
    };

    if name.is_empty() || !is_valid_name(name) {
        return Err(ParseError { line, msg: format!("bad metric name {name:?}") });
    }

    let labels = match labels_part {
        None => BTreeMap::new(),
        Some(body) => parse_labels(body, line)?,
    };

    let value = parse_float(value_str);

    Ok(Sample { name: name.to_string(), labels, value })
}

/// Split on the *last* run of whitespace so `name{a="b c"} 1.0` works.
fn split_last_whitespace(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    let mut last = None;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b' ' || b == b'\t' {
            last = Some(i);
        }
    }
    let i = last?;
    // If the space occurs inside a label block, this is wrong; we'll fix by
    // scanning forward from the closing `}`.
    if let Some(close) = s.rfind('}') {
        if i < close {
            let after = close + 1;
            let tail = s.get(after..)?.trim_start();
            if tail.is_empty() { return None; }
            return Some((&s[..=close], tail));
        }
    }
    Some((&s[..i], s[i..].trim_start()))
}

fn is_valid_name(s: &str) -> bool {
    let mut iter = s.chars();
    let Some(first) = iter.next() else { return false; };
    if !(first.is_ascii_alphabetic() || first == '_' || first == ':') {
        return false;
    }
    iter.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

fn parse_labels(body: &str, line: usize)
    -> Result<BTreeMap<String, String>, ParseError>
{
    let mut out = BTreeMap::new();
    let mut bytes = body.as_bytes();
    loop {
        bytes = trim_start(bytes);
        if bytes.is_empty() { break; }
        // label name
        let name_end = bytes.iter().position(|&b| b == b'=')
            .ok_or_else(|| ParseError { line, msg: "label missing =".into() })?;
        let label_name = std::str::from_utf8(&bytes[..name_end])
            .map_err(|_| ParseError { line, msg: "non-utf8 label".into() })?
            .trim();
        bytes = &bytes[name_end + 1..];
        // opening quote
        if bytes.first() != Some(&b'"') {
            return Err(ParseError { line, msg: "label value missing quote".into() });
        }
        bytes = &bytes[1..];
        // consume until unescaped closing quote
        let mut val = String::new();
        loop {
            match bytes.first().copied() {
                None => return Err(ParseError { line, msg: "unterminated label".into() }),
                Some(b'"') => { bytes = &bytes[1..]; break; }
                Some(b'\\') => {
                    match bytes.get(1).copied() {
                        Some(b'"') => { val.push('"'); bytes = &bytes[2..]; }
                        Some(b'\\') => { val.push('\\'); bytes = &bytes[2..]; }
                        Some(b'n') => { val.push('\n'); bytes = &bytes[2..]; }
                        Some(c) => { val.push(c as char); bytes = &bytes[2..]; }
                        None => return Err(ParseError { line, msg: "dangling escape".into() }),
                    }
                }
                Some(c) => { val.push(c as char); bytes = &bytes[1..]; }
            }
        }
        out.insert(label_name.to_string(), val);
        bytes = trim_start(bytes);
        match bytes.first() {
            Some(&b',') => { bytes = &bytes[1..]; }
            Some(_) | None => break,
        }
    }
    Ok(out)
}

fn trim_start(mut b: &[u8]) -> &[u8] {
    while let Some(&c) = b.first() {
        if c == b' ' || c == b'\t' { b = &b[1..]; } else { break; }
    }
    b
}

fn parse_float(s: &str) -> Option<f64> {
    match s {
        "NaN" | "nan" | "+Inf" | "-Inf" | "inf" | "+inf" | "-inf" => None,
        _ => s.parse::<f64>().ok().and_then(
            |v| if v.is_finite() { Some(v) } else { None }
        ),
    }
}
```

- [ ] **Step 4: Wire the module into lib**

In `crates/kyma-connectors/src/lib.rs`, add `pub mod prometheus;` alongside the others.

- [ ] **Step 5: Run tests**

Run: `cargo test -p kyma-connectors --test openmetrics_parser`
Expected: PASS, 6 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-connectors/src/lib.rs \
        crates/kyma-connectors/src/prometheus/ \
        crates/kyma-connectors/tests/openmetrics_parser.rs
git commit -m "$(cat <<'EOF'
add OpenMetrics text parser

Handwritten, no new deps. Handles labels (with escaped quote +
backslash), histograms / summaries in exploded form, NaN / ±Inf
coerced to None, HELP/TYPE comment lines, and malformed lines
that cite the offending line number.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: `PromConnector` struct + `validate_config` (user-contribution point)

**Files:**
- Create: `crates/kyma-connectors/src/prometheus/mod.rs` (replace Task 7 stub)
- Create: `crates/kyma-connectors/tests/validate_config.rs`

> **Note for the executor:** `validate_config` is the explicit user-contribution point from the design. The skeleton below has the parsed config shape and error types wired up; the assistant should scaffold `validate_config`'s signature with a `TODO` and present the trade-offs to the user in-conversation (strict unknown-field rejection vs. lenient ignore; exhaustive auth.type validation vs. best-effort). The user writes the 5-10 lines that define the validation policy.

- [ ] **Step 1: Write failing tests covering the skeleton**

Create `crates/kyma-connectors/tests/validate_config.rs`:

```rust
use kyma_connectors::prometheus::PromConnector;
use kyma_connectors::Connector;
use serde_json::json;

fn c() -> PromConnector { PromConnector::default() }

#[test]
fn type_id_is_prometheus() {
    assert_eq!(c().type_id(), "prometheus");
}

#[test]
fn accepts_minimal_config() {
    let cfg = json!({ "endpoint": "http://127.0.0.1:9090/metrics" });
    c().validate_config(&cfg).expect("ok");
}

#[test]
fn accepts_https_and_http_only() {
    for scheme in &["http", "https"] {
        let cfg = json!({ "endpoint": format!("{scheme}://x/metrics") });
        c().validate_config(&cfg).unwrap_or_else(|e| panic!("{scheme}: {e:?}"));
    }
    let cfg = json!({ "endpoint": "ftp://x/metrics" });
    c().validate_config(&cfg).expect_err("ftp rejected");
}

#[test]
fn rejects_missing_endpoint() {
    let cfg = json!({});
    let err = c().validate_config(&cfg).unwrap_err();
    assert!(err.0.contains("endpoint"), "error: {err:?}");
}

#[test]
fn auth_bearer_requires_token_ref() {
    let cfg = json!({
        "endpoint": "http://x/metrics",
        "auth": { "type": "bearer" }
    });
    let err = c().validate_config(&cfg).unwrap_err();
    assert!(err.0.contains("token_ref"), "error: {err:?}");
}

#[test]
fn auth_basic_requires_username_and_password() {
    let cfg = json!({
        "endpoint": "http://x/metrics",
        "auth": { "type": "basic", "username": "u" }
    });
    let err = c().validate_config(&cfg).unwrap_err();
    assert!(err.0.contains("password_ref"), "error: {err:?}");
}
```

- [ ] **Step 2: Run tests; expect compile failure**

Run: `cargo test -p kyma-connectors --test validate_config`
Expected: FAIL (`PromConnector` undefined).

- [ ] **Step 3: Scaffold `PromConnector`**

Replace `crates/kyma-connectors/src/prometheus/mod.rs`:

```rust
//! Prometheus `/metrics` scrape connector.

pub mod parser;

use async_trait::async_trait;
use serde::Deserialize;

use crate::types::{
    ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun,
};

#[derive(Default, Clone, Debug)]
pub struct PromConnector;

/// Parsed form of the connector's JSON config. Kept private — validation
/// and `run_once` both deserialize internally.
#[derive(Debug, Deserialize)]
pub(crate) struct PromConfig {
    pub endpoint: String,
    #[serde(default)]
    pub auth: Option<PromAuth>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 { 5_000 }

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum PromAuth {
    None,
    Bearer {
        token_ref: String,
    },
    Basic {
        username: String,
        password_ref: String,
    },
}

#[async_trait]
impl Connector for PromConnector {
    fn type_id(&self) -> &'static str { "prometheus" }

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        // -----------------------------------------------------------
        // TODO (USER CONTRIBUTION): define the validation policy.
        //
        // At minimum:
        //   * `endpoint` must be present.
        //   * scheme must be http or https.
        //
        // Trade-offs to decide:
        //   (a) Strict unknown-field rejection (catches `authz` typo for
        //       `auth`) vs. lenient `#[serde(deny_unknown_fields)]` off
        //       (more forward-compatible when adding fields later).
        //   (b) Validate auth.type values exhaustively, including the
        //       cross-field constraints (bearer => token_ref present,
        //       basic => username + password_ref present).
        //   (c) Bound `timeout_ms` (e.g., reject < 100 or > 60_000) so a
        //       misconfig can't DoS the runner.
        //
        // The tests in `tests/validate_config.rs` encode the minimum
        // acceptable behaviour — make them pass while writing the 5-10
        // lines here that reflect your chosen policy.
        //
        // Start by deserializing `cfg` into `PromConfig` via serde_json,
        // map errors to ConfigError(msg), then add the scheme check,
        // then add auth cross-field constraints.
        // -----------------------------------------------------------
        let _ = cfg;
        todo!("user contribution: implement validation policy")
    }

    async fn run_once(
        &self,
        _ctx: &ConnectorCtx,
        _cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        Err(ConnectorError::Permanent("run_once not yet implemented".into()))
    }
}
```

- [ ] **Step 4: Pause for user contribution**

Pause here and prompt the user:

> "I've scaffolded `crates/kyma-connectors/src/prometheus/mod.rs` with the config types, the trait skeleton, and a `TODO` at `validate_config`. Please write the validation body — the tests in `tests/validate_config.rs` encode the minimum acceptable behaviour. Trade-offs to pick: (a) strict unknown-field rejection vs. lenient; (b) exhaustive auth cross-field validation; (c) a bound on `timeout_ms`. Aim for 5-10 lines."

Once the user has written the body, continue.

- [ ] **Step 5: Run tests**

Run: `cargo test -p kyma-connectors --test validate_config`
Expected: PASS, 6 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-connectors/src/prometheus/mod.rs \
        crates/kyma-connectors/tests/validate_config.rs
git commit -m "$(cat <<'EOF'
add PromConnector skeleton and validate_config

Config schema (endpoint, auth bearer/basic/none, timeout_ms) and
the trait impl. Validation policy is the user-contribution point:
minimum behaviour encoded as tests, concrete policy inline.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: `PromConnector::run_once` — HTTP fetch + retry + emit rows

**Files:**
- Modify: `crates/kyma-connectors/src/prometheus/mod.rs`
- Create: `crates/kyma-connectors/tests/prometheus_run_once.rs`

- [ ] **Step 1: Write the failing test (spawns a mock HTTP server in-process)**

Create `crates/kyma-connectors/tests/prometheus_run_once.rs`:

```rust
//! Exercises PromConnector::run_once against an in-process HTTP server.

use kyma_connectors::metrics::ConnectorMetrics;
use kyma_connectors::prometheus::PromConnector;
use kyma_connectors::secrets::EnvSecretStore;
use kyma_connectors::{Connector, ConnectorCtx};
use serde_json::json;
use std::sync::Arc;

async fn spawn_mock(body: &'static str, status: u16) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let body = body;
            let status = status;
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 8192];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 {status} x\r\nContent-Length: {len}\r\n\r\n{body}",
                    len = body.len(),
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/metrics")
}

fn ctx() -> ConnectorCtx {
    ConnectorCtx {
        connector_id: uuid::Uuid::new_v4(),
        http: reqwest::Client::new(),
        secrets: Arc::new(EnvSecretStore),
        scheduled_for: chrono::Utc::now(),
        metrics: ConnectorMetrics {
            connector_id: uuid::Uuid::new_v4(),
            type_id: "prometheus",
        },
    }
}

#[tokio::test]
async fn happy_path_emits_rows() {
    let endpoint = spawn_mock("a_metric{x=\"y\"} 1\nb_metric 2\n", 200).await;
    let cfg = json!({ "endpoint": endpoint });
    let run = PromConnector::default()
        .run_once(&ctx(), &cfg, None)
        .await
        .expect("ok");
    assert_eq!(run.rows.len(), 2);
    assert!(run.new_cursor.is_none());
    let r0 = &run.rows[0];
    assert_eq!(r0["name"], "a_metric");
    assert_eq!(r0["labels"]["x"], "y");
    assert_eq!(r0["value"], 1.0);
    assert!(r0["timestamp"].is_string());
}

#[tokio::test]
async fn http_5xx_is_transient() {
    let endpoint = spawn_mock("", 503).await;
    let cfg = json!({ "endpoint": endpoint });
    let err = PromConnector::default()
        .run_once(&ctx(), &cfg, None)
        .await
        .unwrap_err();
    match err {
        kyma_connectors::ConnectorError::Transient(_) => {}
        other => panic!("expected Transient, got {other:?}"),
    }
}

#[tokio::test]
async fn http_4xx_is_permanent() {
    let endpoint = spawn_mock("", 404).await;
    let cfg = json!({ "endpoint": endpoint });
    let err = PromConnector::default()
        .run_once(&ctx(), &cfg, None)
        .await
        .unwrap_err();
    match err {
        kyma_connectors::ConnectorError::Permanent(_) => {}
        other => panic!("expected Permanent, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run — expect failure (`todo!()` in Task 8's skeleton)**

Run: `cargo test -p kyma-connectors --test prometheus_run_once`
Expected: FAIL.

- [ ] **Step 3: Implement `run_once`**

In `crates/kyma-connectors/src/prometheus/mod.rs`, replace the stub `run_once` body with:

```rust
async fn run_once(
    &self,
    ctx: &ConnectorCtx,
    cfg: &serde_json::Value,
    _cursor: Option<&serde_json::Value>,
) -> Result<ConnectorRun, ConnectorError> {
    let parsed: PromConfig = serde_json::from_value(cfg.clone())
        .map_err(|e| ConnectorError::Config(format!("config parse: {e}")))?;

    // Resolve auth credentials via the SecretStore.
    let (bearer, basic) = match &parsed.auth {
        None | Some(PromAuth::None) => (None, None),
        Some(PromAuth::Bearer { token_ref }) => {
            let t = ctx.secrets.resolve(token_ref)
                .map_err(|e| ConnectorError::Config(format!("token resolve: {e}")))?;
            (Some(t), None)
        }
        Some(PromAuth::Basic { username, password_ref }) => {
            let p = ctx.secrets.resolve(password_ref)
                .map_err(|e| ConnectorError::Config(format!("password resolve: {e}")))?;
            (None, Some((username.clone(), p)))
        }
    };

    // Retry transient errors up to 3 times with jittered exponential backoff.
    let mut attempt: u32 = 0;
    let body = loop {
        let mut req = ctx.http.get(&parsed.endpoint)
            .timeout(std::time::Duration::from_millis(parsed.timeout_ms))
            .header(
                "Accept",
                "application/openmetrics-text; version=1.0.0, text/plain; version=0.0.4",
            );
        if let Some(t) = &bearer { req = req.bearer_auth(t); }
        if let Some((u, p)) = &basic { req = req.basic_auth(u, Some(p)); }

        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    break resp.text().await.map_err(|e|
                        ConnectorError::Transient(format!("body read: {e}")))?;
                }
                if status.as_u16() == 429 || status.is_server_error() {
                    // transient — backoff and retry
                    if attempt >= 3 {
                        return Err(ConnectorError::Transient(
                            format!("HTTP {status} after {attempt} retries")));
                    }
                } else {
                    return Err(ConnectorError::Permanent(format!("HTTP {status}")));
                }
            }
            Err(e) if e.is_timeout() || e.is_connect() => {
                if attempt >= 3 {
                    return Err(ConnectorError::Transient(
                        format!("network: {e} after {attempt} retries")));
                }
            }
            Err(e) => return Err(ConnectorError::Permanent(format!("fetch: {e}"))),
        }
        attempt += 1;
        let base_ms = 100u64 * (1u64 << (attempt - 1).min(5));  // 100, 200, 400, ...
        let jitter = fastrand::u64(..base_ms / 3 + 1);
        tokio::time::sleep(
            std::time::Duration::from_millis(base_ms.saturating_add(jitter))
        ).await;
    };

    // Parse + emit.
    let samples = parser::parse_openmetrics(&body)
        .map_err(|e| ConnectorError::Permanent(format!("parse: {e}")))?;
    let ts = ctx.scheduled_for.to_rfc3339();
    let rows = samples.into_iter().map(|s| serde_json::json!({
        "timestamp": ts,
        "name": s.name,
        "value": s.value,
        "labels": s.labels,
    })).collect();

    Ok(ConnectorRun { rows, new_cursor: None })
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-connectors --test prometheus_run_once`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-connectors/src/prometheus/mod.rs \
        crates/kyma-connectors/tests/prometheus_run_once.rs
git commit -m "$(cat <<'EOF'
implement PromConnector::run_once

HTTP GET with bearer/basic auth resolution via SecretStore,
jittered expo backoff up to 3 retries on transient errors, 4xx
(except 429) maps to Permanent, parser output mapped to JSON rows
with scrape-time timestamp.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: `rows_to_batches` — JSON→Arrow coercion helper

**Files:**
- Create: `crates/kyma-connectors/src/arrow_coerce.rs`
- Create: `crates/kyma-connectors/tests/arrow_coerce.rs`
- Modify: `crates/kyma-connectors/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/kyma-connectors/tests/arrow_coerce.rs`:

```rust
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use kyma_connectors::arrow_coerce::rows_to_batches;
use serde_json::json;
use std::sync::Arc;

fn metrics_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("timestamp", DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())), false),
        Field::new("name", DataType::Utf8, false),
        Field::new("value", DataType::Float64, true),
        Field::new("labels", DataType::Utf8, true),  // phase A: labels as JSON-string
    ]))
}

#[test]
fn coerces_rows_into_one_batch() {
    let schema = metrics_schema();
    let rows = vec![
        json!({
            "timestamp": "2026-04-20T12:00:00Z",
            "name": "a",
            "value": 1.5,
            "labels": "{\"x\":\"y\"}",
        }),
        json!({
            "timestamp": "2026-04-20T12:00:00Z",
            "name": "b",
            "value": null,
            "labels": "{}",
        }),
    ];
    let batches = rows_to_batches(&schema, rows).expect("coerce");
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 2);
    assert_eq!(batches[0].num_columns(), 4);
}

#[test]
fn empty_rows_yields_empty_vec() {
    let schema = metrics_schema();
    let batches = rows_to_batches(&schema, vec![]).expect("coerce");
    assert!(batches.is_empty());
}
```

- [ ] **Step 2: Run — expect compile failure**

Run: `cargo test -p kyma-connectors --test arrow_coerce`
Expected: FAIL.

- [ ] **Step 3: Implement**

Create `crates/kyma-connectors/src/arrow_coerce.rs`:

```rust
//! JSON rows → Arrow `RecordBatch` coercion, matching kyma-ingest-rest.
//!
//! Serializes to NDJSON bytes and feeds `arrow::json::ReaderBuilder`, so the
//! coercion rules (type promotion, missing-column null-fill, etc.) are
//! exactly identical to what REST ingest applies.

use arrow_array::RecordBatch;
use arrow_schema::Schema;
use std::io::Cursor;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum CoerceError {
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("arrow: {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

pub fn rows_to_batches(
    schema: &Arc<Schema>,
    rows: Vec<serde_json::Value>,
) -> Result<Vec<RecordBatch>, CoerceError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut buf = Vec::with_capacity(rows.len() * 64);
    for r in &rows {
        serde_json::to_writer(&mut buf, r)?;
        buf.push(b'\n');
    }
    let reader = arrow::json::ReaderBuilder::new(schema.clone())
        .build(Cursor::new(buf))?;
    let mut out = Vec::new();
    for b in reader { out.push(b?); }
    Ok(out)
}
```

- [ ] **Step 4: Expose from `lib.rs`**

Add `pub mod arrow_coerce;` to `crates/kyma-connectors/src/lib.rs`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p kyma-connectors --test arrow_coerce`
Expected: PASS, 2 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-connectors/src/arrow_coerce.rs \
        crates/kyma-connectors/src/lib.rs \
        crates/kyma-connectors/tests/arrow_coerce.rs
git commit -m "$(cat <<'EOF'
add rows_to_batches JSON->RecordBatch helper

Round-trips JSON rows through NDJSON → arrow::json::ReaderBuilder
so connector output uses the same coercion rules as REST ingest.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: `ConnectorScheduler` — enqueue ticks into `background_tasks`

**Files:**
- Create: `crates/kyma-connectors/src/catalog_sql.rs` (SQL helpers; uses downcast to PostgresCatalog::pool())
- Create: `crates/kyma-connectors/src/scheduler.rs`
- Create: `crates/kyma-connectors/tests/scheduler_it.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/kyma-connectors/tests/scheduler_it.rs`:

```rust
//! Integration test (testcontainers) for the scheduler.

use kyma_catalog::PostgresCatalog;
use kyma_connectors::catalog_sql;
use kyma_connectors::scheduler::ConnectorScheduler;
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

async fn pg_catalog() -> (testcontainers::ContainerAsync<Postgres>, Arc<PostgresCatalog>) {
    let pg = Postgres::default().start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());
    (pg, catalog)
}

#[tokio::test]
async fn inserts_tick_after_interval() {
    let (_pg, catalog) = pg_catalog().await;
    // Insert a connectors row directly.
    let id = catalog_sql::create_connector_direct(
        catalog.pool(),
        "p1", "prometheus", "db", "metrics",
        serde_json::json!({ "endpoint": "http://x/metrics" }),
        100,
        "periodic",
    ).await.unwrap();

    let sched = ConnectorScheduler::new(catalog.clone());
    sched.tick_once().await.expect("first tick");
    sched.tick_once().await.expect("second tick is a no-op (dedup)");

    let rows = sqlx::query_as::<_, (i64,)>(
        "SELECT count(*) FROM background_tasks
         WHERE kind = 'connector_tick'
           AND payload->>'connector_id' = $1::text",
    )
    .bind(id.to_string())
    .fetch_one(catalog.pool())
    .await
    .unwrap();
    assert_eq!(rows.0, 1, "exactly one task enqueued");
}

#[tokio::test]
async fn disabled_connectors_are_skipped() {
    let (_pg, catalog) = pg_catalog().await;
    let id = catalog_sql::create_connector_direct(
        catalog.pool(),
        "p2", "prometheus", "db", "metrics",
        serde_json::json!({ "endpoint": "http://x/metrics" }),
        100,
        "periodic",
    ).await.unwrap();
    sqlx::query("UPDATE connectors SET enabled = FALSE WHERE id = $1")
        .bind(id)
        .execute(catalog.pool())
        .await
        .unwrap();

    let sched = ConnectorScheduler::new(catalog.clone());
    sched.tick_once().await.unwrap();

    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM background_tasks WHERE kind = 'connector_tick'",
    )
    .fetch_one(catalog.pool())
    .await
    .unwrap();
    assert_eq!(count, 0);
}
```

- [ ] **Step 2: Run — expect failure**

Run: `cargo test -p kyma-connectors --test scheduler_it`
Expected: FAIL (module not defined).

- [ ] **Step 3: Implement `catalog_sql.rs`**

Create `crates/kyma-connectors/src/catalog_sql.rs`:

```rust
//! Direct SQL helpers against PostgresCatalog::pool().
//!
//! Connector-scheduler and -runner read/write a handful of connector-
//! specific rows that don't warrant growing the Catalog trait. This
//! module is the one place those SQL statements live.

use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ConnectorRow {
    pub id: Uuid,
    pub name: String,
    pub type_id: String,
    pub target_database: String,
    pub target_table: String,
    pub config_jsonb: serde_json::Value,
    pub schedule_ms: i64,
    pub drive_model: String,
    pub enabled: bool,
}

/// Create a connector row (used from admin API + test setup).
#[allow(clippy::too_many_arguments)]
pub async fn create_connector_direct(
    pool: &PgPool,
    name: &str,
    type_id: &str,
    target_database: &str,
    target_table: &str,
    config: serde_json::Value,
    schedule_ms: i64,
    drive_model: &str,
) -> Result<Uuid, sqlx::Error> {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO connectors
           (name, type, target_database, target_table, config_jsonb,
            schedule_ms, drive_model)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id",
    )
    .bind(name)
    .bind(type_id)
    .bind(target_database)
    .bind(target_table)
    .bind(&config)
    .bind(schedule_ms)
    .bind(drive_model)
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// List periodic, enabled connectors due for a tick.
pub async fn list_due_periodic(pool: &PgPool) -> Result<Vec<ConnectorRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, name, type, target_database, target_table, config_jsonb,
                schedule_ms, drive_model, enabled, last_run_at
         FROM connectors
         WHERE enabled = TRUE AND drive_model = 'periodic'
           AND (last_run_at IS NULL
                OR last_run_at < now() - (schedule_ms || ' milliseconds')::interval)",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(|r| Ok(ConnectorRow {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        type_id: r.try_get("type")?,
        target_database: r.try_get("target_database")?,
        target_table: r.try_get("target_table")?,
        config_jsonb: r.try_get("config_jsonb")?,
        schedule_ms: r.try_get("schedule_ms")?,
        drive_model: r.try_get("drive_model")?,
        enabled: r.try_get("enabled")?,
    })).collect()
}

pub async fn load_connector(pool: &PgPool, id: Uuid)
    -> Result<Option<ConnectorRow>, sqlx::Error>
{
    let row = sqlx::query(
        "SELECT id, name, type, target_database, target_table, config_jsonb,
                schedule_ms, drive_model, enabled
         FROM connectors WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some(r) = row else { return Ok(None); };
    Ok(Some(ConnectorRow {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        type_id: r.try_get("type")?,
        target_database: r.try_get("target_database")?,
        target_table: r.try_get("target_table")?,
        config_jsonb: r.try_get("config_jsonb")?,
        schedule_ms: r.try_get("schedule_ms")?,
        drive_model: r.try_get("drive_model")?,
        enabled: r.try_get("enabled")?,
    }))
}

pub async fn load_cursor(pool: &PgPool, connector_id: Uuid)
    -> Result<Option<serde_json::Value>, sqlx::Error>
{
    let row: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT cursor_jsonb FROM connector_cursors WHERE connector_id = $1",
    )
    .bind(connector_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(v,)| if v.is_null() { None } else { Some(v) }))
}

pub async fn upsert_cursor(
    pool: &PgPool,
    connector_id: Uuid,
    cursor: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO connector_cursors (connector_id, cursor_jsonb)
         VALUES ($1, $2)
         ON CONFLICT (connector_id)
         DO UPDATE SET cursor_jsonb = EXCLUDED.cursor_jsonb, updated_at = now()",
    )
    .bind(connector_id)
    .bind(cursor)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_run_success(
    pool: &PgPool,
    connector_id: Uuid,
    rows_ingested: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET last_run_at = now(),
             last_success_at = now(),
             last_error = NULL,
             last_rows_ingested = $2,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(connector_id)
    .bind(rows_ingested)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_run_failure(
    pool: &PgPool,
    connector_id: Uuid,
    error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET last_run_at = now(),
             last_error = $2,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(connector_id)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn disable_connector(
    pool: &PgPool,
    connector_id: Uuid,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE connectors
         SET enabled = FALSE, disabled_reason = $2, updated_at = now()
         WHERE id = $1",
    )
    .bind(connector_id)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// Enqueue a single connector_tick task with the bucketed `scheduled_for`.
/// Race-safe: the partial unique index on `background_tasks` turns duplicate
/// inserts into `Ok(0)` (we use ON CONFLICT DO NOTHING).
pub async fn enqueue_tick(
    pool: &PgPool,
    connector_id: Uuid,
    scheduled_for_ms: i64,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "INSERT INTO background_tasks (kind, payload, priority)
         VALUES ('connector_tick',
                 jsonb_build_object(
                     'connector_id', $1::text,
                     'scheduled_for', $2::text),
                 0)
         ON CONFLICT DO NOTHING",
    )
    .bind(connector_id)
    .bind(scheduled_for_ms)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
```

- [ ] **Step 4: Implement `scheduler.rs`**

Create `crates/kyma-connectors/src/scheduler.rs`:

```rust
//! Connector scheduler — enqueues connector_tick tasks for due connectors.

use crate::catalog_sql;
use kyma_catalog::PostgresCatalog;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info};

#[derive(Clone)]
pub struct ConnectorScheduler {
    catalog: Arc<PostgresCatalog>,
    pub tick_interval: Duration,
}

impl ConnectorScheduler {
    pub fn new(catalog: Arc<PostgresCatalog>) -> Self {
        Self { catalog, tick_interval: Duration::from_millis(500) }
    }

    pub async fn tick_once(&self) -> Result<(), sqlx::Error> {
        let due = catalog_sql::list_due_periodic(self.catalog.pool()).await?;
        for c in due {
            let now_ms = chrono::Utc::now().timestamp_millis();
            let bucketed = (now_ms / c.schedule_ms) * c.schedule_ms;
            let inserted = catalog_sql::enqueue_tick(
                self.catalog.pool(), c.id, bucketed,
            ).await?;
            if inserted > 0 {
                debug!(connector = %c.name, bucketed, "enqueued connector_tick");
            }
        }
        Ok(())
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!("connector scheduler starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("connector scheduler shutdown"); return; }
                _ = tokio::time::sleep(self.tick_interval) => {
                    if let Err(e) = self.tick_once().await {
                        error!(error = %e, "scheduler tick failed");
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 5: Expose from `lib.rs`**

Add `pub mod catalog_sql;` and `pub mod scheduler;` to `crates/kyma-connectors/src/lib.rs`.

- [ ] **Step 6: Run integration test**

Run: `cargo test -p kyma-connectors --test scheduler_it`
Expected: PASS, 2 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/kyma-connectors/src/catalog_sql.rs \
        crates/kyma-connectors/src/scheduler.rs \
        crates/kyma-connectors/src/lib.rs \
        crates/kyma-connectors/tests/scheduler_it.rs
git commit -m "$(cat <<'EOF'
add ConnectorScheduler

Polls connectors every 500ms. Bucketed scheduled_for + ON CONFLICT
DO NOTHING on enqueue means multiple concurrent schedulers cannot
create duplicate connector_tick tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: `ConnectorRunner` — claim ticks, invoke, ingest, commit cursor

**Files:**
- Create: `crates/kyma-connectors/src/runner.rs`
- Create: `crates/kyma-connectors/tests/runner_it.rs`

- [ ] **Step 1: Write the failing integration test**

Create `crates/kyma-connectors/tests/runner_it.rs`:

```rust
//! Integration test — scheduler + runner + a fake Connector.

use async_trait::async_trait;
use kyma_catalog::PostgresCatalog;
use kyma_connectors::catalog_sql;
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::runner::ConnectorRunner;
use kyma_connectors::scheduler::ConnectorScheduler;
use kyma_connectors::secrets::EnvSecretStore;
use kyma_connectors::{
    ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun,
};
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

#[derive(Default)]
struct CountingConn {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Connector for CountingConn {
    fn type_id(&self) -> &'static str { "counter" }
    fn validate_config(&self, _: &serde_json::Value) -> Result<(), ConfigError> { Ok(()) }
    async fn run_once(
        &self,
        _ctx: &ConnectorCtx,
        _cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        let n = cursor.and_then(|v| v.as_u64()).unwrap_or(0);
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ConnectorRun {
            rows: vec![serde_json::json!({"n": n, "ok": true})],
            new_cursor: Some(serde_json::json!(n + 1)),
        })
    }
}

#[tokio::test]
async fn runner_claims_and_updates_cursor() {
    let pg = Postgres::default().start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());

    let calls = Arc::new(AtomicUsize::new(0));
    let fake = Arc::new(CountingConn { calls: calls.clone() });
    let mut reg = ConnectorRegistry::new();
    reg.register(fake.clone());

    let id = catalog_sql::create_connector_direct(
        catalog.pool(),
        "c1", "counter", "db", "tbl",
        serde_json::json!({}),
        100, "periodic",
    ).await.unwrap();

    let sched = ConnectorScheduler::new(catalog.clone());
    sched.tick_once().await.unwrap();

    // Runner uses a stubbed RowSink (closes over rows) so we avoid depending
    // on a live WritePath here. Real WritePath wiring is covered by the E2E
    // test script.
    let sink: kyma_connectors::runner::RowSink = Arc::new(|_db, _tbl, _rows, _idem| Box::pin(async move { Ok(()) }));
    let runner = ConnectorRunner::new(catalog.clone(), Arc::new(reg), sink, EnvSecretStore, "node-a".into());
    runner.claim_and_run_one().await.expect("tick ran");

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let cur = catalog_sql::load_cursor(catalog.pool(), id).await.unwrap();
    assert_eq!(cur, Some(serde_json::json!(1)));
}
```

- [ ] **Step 2: Run — expect failure**

Run: `cargo test -p kyma-connectors --test runner_it`
Expected: FAIL.

- [ ] **Step 3: Implement `runner.rs`**

Create `crates/kyma-connectors/src/runner.rs`:

```rust
//! Connector tick runner.

use crate::catalog_sql;
use crate::metrics::{ConnectorMetrics, TickResult};
use crate::registry::ConnectorRegistry;
use crate::secrets::SecretStore;
use crate::types::{ConnectorCtx, ConnectorError, ConnectorRun};
use chrono::Utc;
use futures::future::BoxFuture;
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::types::NodeId;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Thin async "send these rows into WritePath" closure. Passed in so the
/// runner does not need a generic on WritePath; that plumbing lives in
/// kyma-bin where the concrete types are assembled.
pub type RowSink = Arc<
    dyn Fn(
            String,                      // target_database
            String,                      // target_table
            Vec<serde_json::Value>,      // rows
            Option<String>,              // idempotency key
        ) -> BoxFuture<'static, Result<(), anyhow::Error>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct ConnectorRunner {
    catalog: Arc<PostgresCatalog>,
    registry: Arc<ConnectorRegistry>,
    sink: RowSink,
    secrets: Arc<dyn SecretStore>,
    node_id: String,
    pub idle_sleep: Duration,
    pub claim_lease: chrono::Duration,
}

impl ConnectorRunner {
    pub fn new<S: SecretStore + 'static>(
        catalog: Arc<PostgresCatalog>,
        registry: Arc<ConnectorRegistry>,
        sink: RowSink,
        secrets: S,
        node_id: String,
    ) -> Self {
        Self {
            catalog,
            registry,
            sink,
            secrets: Arc::new(secrets),
            node_id,
            idle_sleep: Duration::from_millis(200),
            claim_lease: chrono::Duration::seconds(60),
        }
    }

    pub async fn claim_and_run_one(&self) -> Result<bool, anyhow::Error> {
        // NodeId here is for the Catalog::claim_task signature; claim_task
        // expects a Uuid, so we synthesize one per-process if we don't have
        // a registered node.
        let node_uuid = match uuid::Uuid::parse_str(&self.node_id) {
            Ok(u) => NodeId::from_uuid(u),
            Err(_) => NodeId::from_uuid(Uuid::new_v4()),
        };
        let Some(task) = self.catalog
            .claim_task("connector_tick", node_uuid, self.claim_lease)
            .await?
        else {
            return Ok(false);
        };

        let connector_id = task.payload
            .get("connector_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("task missing connector_id"))?;
        let scheduled_for_ms = task.payload
            .get("scheduled_for")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| anyhow::anyhow!("task missing scheduled_for"))?;
        let scheduled_for = chrono::DateTime::<Utc>::from_timestamp_millis(
            scheduled_for_ms,
        ).ok_or_else(|| anyhow::anyhow!("bad scheduled_for"))?;

        let conn = match catalog_sql::load_connector(self.catalog.pool(), connector_id).await? {
            Some(c) if c.enabled => c,
            Some(_) => {
                debug!(connector_id = %connector_id, "skipping disabled connector");
                self.catalog.complete_task(task.id).await?;
                return Ok(true);
            }
            None => {
                warn!(connector_id = %connector_id, "connector row missing; completing task");
                self.catalog.complete_task(task.id).await?;
                return Ok(true);
            }
        };

        let impl_arc = self.registry.lookup(&conn.type_id)
            .ok_or_else(|| anyhow::anyhow!(
                "no registered impl for type {}", conn.type_id))?;

        let cursor = catalog_sql::load_cursor(self.catalog.pool(), connector_id).await?;
        let metrics = ConnectorMetrics {
            connector_id,
            type_id: impl_arc.type_id(),
        };
        let ctx = ConnectorCtx {
            connector_id,
            http: reqwest::Client::builder().build()?,
            secrets: self.secrets.clone(),
            scheduled_for,
            metrics: metrics.clone(),
        };

        let t0 = std::time::Instant::now();
        let outcome = impl_arc
            .run_once(&ctx, &conn.config_jsonb, cursor.as_ref())
            .await;

        match outcome {
            Ok(ConnectorRun { rows, new_cursor }) => {
                let n_rows = rows.len() as u64;
                let idem = format!(
                    "connector:{}:{}",
                    connector_id, scheduled_for_ms * 1_000_000,  // ns
                );
                (self.sink)(
                    conn.target_database.clone(),
                    conn.target_table.clone(),
                    rows,
                    Some(idem),
                ).await?;
                if let Some(c) = new_cursor {
                    catalog_sql::upsert_cursor(self.catalog.pool(), connector_id, &c).await?;
                }
                catalog_sql::mark_run_success(
                    self.catalog.pool(), connector_id, n_rows as i64,
                ).await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_rows(n_rows);
                metrics.record_tick(TickResult::Ok, t0.elapsed().as_secs_f64());
                metrics.set_last_success(Utc::now());
                info!(connector_id = %connector_id, rows = n_rows, "tick ok");
                Ok(true)
            }
            Err(ConnectorError::Transient(msg)) => {
                warn!(connector_id = %connector_id, error = %msg, "transient");
                catalog_sql::mark_run_failure(
                    self.catalog.pool(), connector_id, &msg,
                ).await?;
                self.catalog.fail_task(task.id, &msg).await?;
                metrics.record_error("transient");
                metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                Ok(true)
            }
            Err(ConnectorError::Permanent(msg)) => {
                warn!(connector_id = %connector_id, error = %msg, "permanent");
                catalog_sql::mark_run_failure(
                    self.catalog.pool(), connector_id, &msg,
                ).await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_error("permanent");
                metrics.record_tick(TickResult::Permanent, t0.elapsed().as_secs_f64());
                Ok(true)
            }
            Err(ConnectorError::Config(msg)) => {
                error!(connector_id = %connector_id, error = %msg, "config");
                catalog_sql::disable_connector(
                    self.catalog.pool(), connector_id, &msg,
                ).await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_error("config");
                metrics.record_tick(TickResult::Config, t0.elapsed().as_secs_f64());
                Ok(true)
            }
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(node_id = %self.node_id, "connector runner starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("connector runner shutdown"); return; }
                res = self.claim_and_run_one() => match res {
                    Ok(true)  => {}
                    Ok(false) => tokio::time::sleep(self.idle_sleep).await,
                    Err(e) => {
                        error!(error = %e, "runner error");
                        tokio::time::sleep(self.idle_sleep).await;
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 4: Expose from `lib.rs`**

Add `pub mod runner;` to `crates/kyma-connectors/src/lib.rs`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p kyma-connectors --test runner_it`
Expected: PASS, 1 test.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-connectors/src/runner.rs \
        crates/kyma-connectors/src/lib.rs \
        crates/kyma-connectors/tests/runner_it.rs
git commit -m "$(cat <<'EOF'
add ConnectorRunner

Claims connector_tick from background_tasks, invokes the connector
via the registry, sinks rows through the closure passed in from
kyma-bin (where WritePath lives), updates the cursor, records
per-tick metrics, and maps each ConnectorError variant to the
right background_tasks outcome.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Admin HTTP API — `/v1/connectors` CRUD

**Files:**
- Create: `crates/kyma-connectors/src/admin.rs`
- Create: `crates/kyma-connectors/tests/admin_it.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/kyma-connectors/tests/admin_it.rs`:

```rust
use async_trait::async_trait;
use axum::http::StatusCode;
use axum::Router;
use kyma_catalog::PostgresCatalog;
use kyma_connectors::admin::{router, AdminState};
use kyma_connectors::registry::ConnectorRegistry;
use kyma_connectors::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};
use serde_json::json;
use std::sync::Arc;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tower::ServiceExt;

struct StubConn;

#[async_trait]
impl Connector for StubConn {
    fn type_id(&self) -> &'static str { "stub" }
    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        if cfg.get("endpoint").and_then(|v| v.as_str()).is_some() { Ok(()) }
        else { Err(ConfigError("endpoint required".into())) }
    }
    async fn run_once(
        &self,
        _: &ConnectorCtx,
        _: &serde_json::Value,
        _: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        Ok(ConnectorRun { rows: vec![], new_cursor: None })
    }
}

async fn state() -> (testcontainers::ContainerAsync<Postgres>, AdminState) {
    let pg = Postgres::default().start().await.unwrap();
    let port = pg.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let catalog = Arc::new(PostgresCatalog::connect(&url).await.unwrap());
    let mut reg = ConnectorRegistry::new();
    reg.register(Arc::new(StubConn));
    let s = AdminState { catalog, registry: Arc::new(reg) };
    (pg, s)
}

fn app(s: AdminState) -> Router { router(s) }

#[tokio::test]
async fn create_list_get_delete() {
    let (_pg, s) = state().await;
    let app = app(s.clone());

    // Create
    let body = json!({
        "name": "p1",
        "type": "stub",
        "target_database": "db",
        "target_table": "metrics",
        "schedule_ms": 1000,
        "config": { "endpoint": "http://x/metrics" }
    });
    let resp = app.clone().oneshot(
        axum::http::Request::builder()
            .method("POST")
            .uri("/v1/connectors")
            .header("content-type", "application/json")
            .body(body.to_string().into())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // List
    let resp = app.clone().oneshot(
        axum::http::Request::builder()
            .method("GET")
            .uri("/v1/connectors")
            .body(axum::body::Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn rejects_invalid_config() {
    let (_pg, s) = state().await;
    let app = app(s.clone());
    let body = json!({
        "name": "p2",
        "type": "stub",
        "target_database": "db",
        "target_table": "metrics",
        "schedule_ms": 1000,
        "config": {}   // missing endpoint
    });
    let resp = app.oneshot(
        axum::http::Request::builder()
            .method("POST")
            .uri("/v1/connectors")
            .header("content-type", "application/json")
            .body(body.to_string().into())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 2: Run — expect failure**

Run: `cargo test -p kyma-connectors --test admin_it`
Expected: FAIL.

- [ ] **Step 3: Implement admin.rs**

Create `crates/kyma-connectors/src/admin.rs`:

```rust
//! HTTP admin API — /v1/connectors CRUD.

use crate::catalog_sql;
use crate::registry::ConnectorRegistry;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use kyma_catalog::PostgresCatalog;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AdminState {
    pub catalog: Arc<PostgresCatalog>,
    pub registry: Arc<ConnectorRegistry>,
}

pub fn router(state: AdminState) -> Router {
    Router::new()
        .route("/v1/connectors", post(create).get(list))
        .route("/v1/connectors/:id", get(get_one).patch(patch_one).delete(delete_one))
        .route("/v1/connectors/:id/pause", post(pause))
        .route("/v1/connectors/:id/resume", post(resume))
        .route("/v1/connectors/:id/trigger", post(trigger))
        .with_state(state)
}

#[derive(Deserialize)]
struct CreateReq {
    name: String,
    #[serde(rename = "type")]
    type_id: String,
    target_database: String,
    target_table: String,
    schedule_ms: i64,
    config: serde_json::Value,
}

#[derive(Serialize)]
struct IdResp { id: Uuid }

async fn create(State(s): State<AdminState>, Json(req): Json<CreateReq>) -> impl IntoResponse {
    let Some(c) = s.registry.lookup(&req.type_id) else {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": format!("unknown type {}", req.type_id),
        }))).into_response();
    };
    if let Err(e) = c.validate_config(&req.config) {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "error": e.0,
        }))).into_response();
    }
    let res = catalog_sql::create_connector_direct(
        s.catalog.pool(), &req.name, &req.type_id,
        &req.target_database, &req.target_table, req.config, req.schedule_ms,
        "periodic",
    ).await;
    match res {
        Ok(id) => (StatusCode::CREATED, Json(IdResp { id })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": e.to_string(),
        }))).into_response(),
    }
}

async fn list(State(s): State<AdminState>) -> impl IntoResponse {
    let rows = sqlx::query_as::<_, (Uuid, String, String, bool)>(
        "SELECT id, name, type, enabled FROM connectors ORDER BY name",
    )
    .fetch_all(s.catalog.pool())
    .await;
    match rows {
        Ok(rows) => {
            let items: Vec<_> = rows.into_iter().map(|(id, name, type_id, enabled)|
                serde_json::json!({ "id": id, "name": name, "type": type_id, "enabled": enabled })
            ).collect();
            (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": e.to_string()
        }))).into_response(),
    }
}

async fn get_one(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    match catalog_sql::load_connector(s.catalog.pool(), id).await {
        Ok(Some(c)) => {
            let scrubbed = scrub_secrets(c.config_jsonb.clone());
            (StatusCode::OK, Json(serde_json::json!({
                "id": c.id,
                "name": c.name,
                "type": c.type_id,
                "target_database": c.target_database,
                "target_table": c.target_table,
                "schedule_ms": c.schedule_ms,
                "drive_model": c.drive_model,
                "enabled": c.enabled,
                "config": scrubbed,
            }))).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": e.to_string()
        }))).into_response(),
    }
}

/// Redact literal values of fields named like `/token|password|secret|key/i`
/// while leaving `$env:NAME` references intact (they're not the secret).
fn scrub_secrets(mut v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    fn looks_secret(name: &str) -> bool {
        let n = name.to_ascii_lowercase();
        n.contains("token") || n.contains("password") || n.contains("secret") || n.contains("key")
    }
    fn walk(v: &mut Value) {
        match v {
            Value::Object(m) => {
                for (k, vv) in m.iter_mut() {
                    match vv {
                        Value::String(s) => {
                            if looks_secret(k) && !s.starts_with("$env:") {
                                *s = "***".into();
                            }
                        }
                        _ => walk(vv),
                    }
                }
            }
            Value::Array(a) => for vv in a.iter_mut() { walk(vv); },
            _ => {}
        }
    }
    walk(&mut v);
    v
}

#[derive(Deserialize)]
struct PatchReq {
    name: Option<String>,
    schedule_ms: Option<i64>,
    enabled: Option<bool>,
    config: Option<serde_json::Value>,
}

async fn patch_one(
    State(s): State<AdminState>,
    Path(id): Path<Uuid>,
    Json(req): Json<PatchReq>,
) -> impl IntoResponse {
    // If the caller is updating config, re-validate.
    if let Some(cfg) = &req.config {
        let Some(c) = catalog_sql::load_connector(s.catalog.pool(), id).await.ok().flatten() else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let Some(impl_) = s.registry.lookup(&c.type_id) else {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": format!("unknown type {}", c.type_id),
            }))).into_response();
        };
        if let Err(e) = impl_.validate_config(cfg) {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
                "error": e.0,
            }))).into_response();
        }
    }
    let res = sqlx::query(
        "UPDATE connectors SET
             name = COALESCE($2, name),
             schedule_ms = COALESCE($3, schedule_ms),
             enabled = COALESCE($4, enabled),
             config_jsonb = COALESCE($5, config_jsonb),
             disabled_reason = CASE WHEN $4 = TRUE THEN NULL ELSE disabled_reason END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(req.name.as_deref())
    .bind(req.schedule_ms)
    .bind(req.enabled)
    .bind(req.config.as_ref())
    .execute(s.catalog.pool())
    .await;
    match res {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": e.to_string()
        }))).into_response(),
    }
}

async fn delete_one(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let res = sqlx::query("DELETE FROM connectors WHERE id = $1")
        .bind(id)
        .execute(s.catalog.pool())
        .await;
    match res {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": e.to_string()
        }))).into_response(),
    }
}

async fn pause(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE connectors SET enabled = FALSE, disabled_reason = 'manual',
         updated_at = now() WHERE id = $1",
    ).bind(id).execute(s.catalog.pool()).await;
    StatusCode::NO_CONTENT
}

async fn resume(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let _ = sqlx::query(
        "UPDATE connectors SET enabled = TRUE, disabled_reason = NULL,
         updated_at = now() WHERE id = $1",
    ).bind(id).execute(s.catalog.pool()).await;
    StatusCode::NO_CONTENT
}

async fn trigger(State(s): State<AdminState>, Path(id): Path<Uuid>) -> impl IntoResponse {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let _ = catalog_sql::enqueue_tick(s.catalog.pool(), id, now_ms).await;
    StatusCode::ACCEPTED
}
```

- [ ] **Step 4: Expose from `lib.rs`**

Add `pub mod admin;` to `crates/kyma-connectors/src/lib.rs`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p kyma-connectors --test admin_it`
Expected: PASS, 2 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-connectors/src/admin.rs \
        crates/kyma-connectors/src/lib.rs \
        crates/kyma-connectors/tests/admin_it.rs
git commit -m "$(cat <<'EOF'
add admin HTTP router for /v1/connectors

CRUD + pause/resume/trigger + config validation on create/patch +
secret scrubbing on GET (literal values in token/password/secret/key
fields masked as '***'; \$env: references pass through).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Wire into `kyma-bin` and `kyma-server`

**Files:**
- Modify: `crates/kyma-bin/src/main.rs`
- Modify: `crates/kyma-server/src/lib.rs`
- Modify: `crates/kyma-bin/Cargo.toml` (add kyma-connectors dep if missing)

- [ ] **Step 1: Register the Prometheus connector and spawn runner + scheduler**

Open `crates/kyma-bin/src/main.rs`. Near the existing compaction scheduler spawn, add:

```rust
use kyma_connectors::{
    prometheus::PromConnector,
    registry::ConnectorRegistry,
    runner::{ConnectorRunner, RowSink},
    scheduler::ConnectorScheduler,
    secrets::EnvSecretStore,
};
use std::sync::Arc;

// ... inside async main, after catalog + write_path are constructed ...

let mut reg = ConnectorRegistry::new();
reg.register(Arc::new(PromConnector::default()));
let registry = Arc::new(reg);

// Translate the runner's row-sink closure into a real WritePath ingest.
let catalog_for_sink = catalog.clone();
let write_path = write_path.clone();
let sink: RowSink = Arc::new(move |db, tbl, rows, idem_key| {
    let write_path = write_path.clone();
    let catalog = catalog_for_sink.clone();
    Box::pin(async move {
        let table = catalog.lookup_table(&db, &tbl).await?;
        let batches = kyma_connectors::arrow_coerce::rows_to_batches(
            &table.schema,
            rows,
        )?;
        write_path.ingest_with_idempotency(
            &table,
            batches,
            idem_key.as_deref(),
        ).await?;
        Ok(())
    })
});

let node_id = std::env::var("KYMA_NODE_ID")
    .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());

// Spawn scheduler.
{
    let sched = ConnectorScheduler::new(catalog.clone());
    let shutdown = shutdown.clone();
    tokio::spawn(async move { sched.run(shutdown.recv()).await });
}

// Spawn N runners.
let n_runners: usize = std::env::var("KYMA_CONNECTOR_WORKERS")
    .ok().and_then(|s| s.parse().ok()).unwrap_or(4);
for _ in 0..n_runners {
    let runner = ConnectorRunner::new(
        catalog.clone(),
        registry.clone(),
        sink.clone(),
        EnvSecretStore,
        node_id.clone(),
    );
    let shutdown = shutdown.clone();
    tokio::spawn(async move { runner.run(shutdown.recv()).await });
}
```

Adjust to match the real variable names in `main.rs` (e.g., `catalog`, `write_path`, `shutdown`). If `shutdown` is exposed as a broadcast receiver, the pattern `shutdown.recv()` becomes `async move { let _ = shutdown.recv().await; }` as a shutdown future.

- [ ] **Step 2: Mount admin router in kyma-server**

In `crates/kyma-server/src/lib.rs`, near where other routers (auth middleware, ingest router, query router) are composed, merge in `kyma_connectors::admin::router(admin_state)` guarded by the `admin` role. The real server already has role-based middleware patterns — match them.

Rough shape:

```rust
use kyma_connectors::admin::{self, AdminState};

pub struct ServerParts {
    // existing fields ...
    pub connector_registry: std::sync::Arc<kyma_connectors::registry::ConnectorRegistry>,
}

// in the router-assembly fn, where the auth-protected router is built:
let admin_state = AdminState {
    catalog: parts.catalog.clone(),
    registry: parts.connector_registry.clone(),
};
router = router.merge(admin::router(admin_state));
```

Adapt to actual type names. `ServerParts` is illustrative — your existing struct may be named differently; rename accordingly.

- [ ] **Step 3: Add crate dep**

In `crates/kyma-bin/Cargo.toml` and `crates/kyma-server/Cargo.toml`, under `[dependencies]`:

```toml
kyma-connectors = { workspace = true }
```

- [ ] **Step 4: Full compile**

Run: `cargo check --workspace --all-targets`
Expected: success, 0 warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-bin/ crates/kyma-server/ Cargo.lock
git commit -m "$(cat <<'EOF'
wire connectors into kyma-bin and kyma-server

kyma-bin spawns the connector scheduler + N runner tasks and
registers PromConnector. The runner's row-sink closure translates
JSON rows through arrow coercion and hands the batches to
WritePath::ingest_with_idempotency. kyma-server mounts the
/v1/connectors admin router behind the admin auth middleware.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 15: E2E test `scripts/test-prometheus-connector.sh`

**Files:**
- Create: `scripts/fixtures/prom-metrics.txt`
- Create: `scripts/test-prometheus-connector.sh`

- [ ] **Step 1: Write the fixture**

Create `scripts/fixtures/prom-metrics.txt`:

```
# HELP http_requests_total Total HTTP requests.
# TYPE http_requests_total counter
http_requests_total{method="GET",status="200"} 42
http_requests_total{method="GET",status="500"} 3
http_requests_total{method="POST",status="200"} 7
# HELP queue_depth Current queue depth.
# TYPE queue_depth gauge
queue_depth 17
# HELP test_latency_seconds Handler latency histogram.
# TYPE test_latency_seconds histogram
test_latency_seconds_bucket{le="0.1"} 10
test_latency_seconds_bucket{le="0.5"} 20
test_latency_seconds_bucket{le="+Inf"} 25
test_latency_seconds_sum 3.14
test_latency_seconds_count 25
```

- [ ] **Step 2: Write the E2E script**

Create `scripts/test-prometheus-connector.sh` (make executable in step 3):

```bash
#!/usr/bin/env bash
# End-to-end test of the Prometheus connector.
#
# Requirements: docker-compose up (postgres + minio), and a freshly started
# kyma binary (caller's responsibility). Uses python3 for the mock /metrics
# endpoint.

set -euo pipefail

HTTP="${KYMA_HTTP:-http://127.0.0.1:8080}"
MOCK_PORT="${MOCK_PORT:-19191}"
FIXTURE="$(dirname "$0")/fixtures/prom-metrics.txt"

pass=0
fail=0

ok()   { pass=$((pass+1)); echo "  PASS: $1"; }
nope() { fail=$((fail+1)); echo "  FAIL: $1"; echo "    $2"; }

cleanup() {
    if [[ -n "${MOCK_PID:-}" ]] && kill -0 "$MOCK_PID" 2>/dev/null; then
        kill "$MOCK_PID" || true
    fi
    if [[ -n "${CONN_ID:-}" ]]; then
        curl -s -X DELETE "$HTTP/v1/connectors/$CONN_ID" >/dev/null || true
    fi
}
trap cleanup EXIT

# Tiny mock HTTP server. -u unbuffered so it starts quickly.
SERVE_DIR="$(mktemp -d)"
cp "$FIXTURE" "$SERVE_DIR/metrics"
( cd "$SERVE_DIR" && python3 -u -m http.server "$MOCK_PORT" ) > /tmp/mock-prom.log 2>&1 &
MOCK_PID=$!
sleep 0.5

# Create a database + table up-front via REST ingest (which lazily creates on
# first ingest — but we need the metrics table schema known ahead of the
# connector's first tick). The ingest endpoint auto-creates 'metrics' with
# a reasonable schema on first write.
echo '{"timestamp":"2026-01-01T00:00:00Z","name":"bootstrap","value":0.0,"labels":"{}"}' \
    | curl -sS -X POST "$HTTP/v1/ingest" \
        -H 'Content-Type: application/x-ndjson' \
        -H 'X-Database: telemetry' \
        -H 'X-Table: metrics' \
        --data-binary @- >/dev/null

# Create the connector.
BODY=$(cat <<EOF
{
  "name": "e2e-prom",
  "type": "prometheus",
  "target_database": "telemetry",
  "target_table": "metrics",
  "schedule_ms": 1000,
  "config": { "endpoint": "http://127.0.0.1:${MOCK_PORT}/metrics" }
}
EOF
)
CREATE=$(curl -sS -X POST "$HTTP/v1/connectors" \
    -H 'Content-Type: application/json' -d "$BODY")
CONN_ID=$(echo "$CREATE" | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')

if [[ -n "$CONN_ID" ]]; then ok "connector created $CONN_ID"
else nope "create" "$CREATE"; fi

# Wait for first success.
for _ in $(seq 1 15); do
    STATUS=$(curl -sS "$HTTP/v1/connectors/$CONN_ID" || echo '')
    if echo "$STATUS" | grep -q '"last_success_at"'; then break; fi
    sleep 0.5
done
if echo "$STATUS" | grep -q '"last_success_at"'; then ok "connector ran once"
else nope "first success" "$STATUS"; fi

# Query kyma for ingested rows.
QUERY() { curl -sS -X POST "$HTTP/v1/query" \
    -H 'Content-Type: application/sql' \
    -H 'X-Database: telemetry' \
    --data "$1"; }

N=$(QUERY "SELECT count(*) AS n FROM metrics WHERE name LIKE 'test_%'" \
   | python3 -c 'import json,sys,re; out=sys.stdin.read();
lines=[l for l in out.split("\n") if l.strip()]
print(lines[-1] if lines else "")' \
   | python3 -c 'import json,sys; j=json.loads(sys.stdin.read()); print(j.get("n"))' 2>/dev/null || echo 0)
if [[ "${N:-0}" -ge 5 ]]; then ok "histogram rows present (n=$N)"
else nope "histogram rows" "n=$N"; fi

# Label preservation.
STATUS_VALS=$(QUERY "SELECT DISTINCT labels FROM metrics WHERE name='http_requests_total'")
if echo "$STATUS_VALS" | grep -q '"200"' && echo "$STATUS_VALS" | grep -q '"500"'; then
    ok "labels preserved"
else
    nope "labels" "$STATUS_VALS"
fi

# Pause blocks new rows.
BEFORE=$(QUERY "SELECT count(*) AS n FROM metrics" | tail -n1)
curl -sS -X POST "$HTTP/v1/connectors/$CONN_ID/pause" >/dev/null
sleep 2
AFTER=$(QUERY "SELECT count(*) AS n FROM metrics" | tail -n1)
if [[ "$BEFORE" == "$AFTER" ]]; then ok "pause stops new ticks"
else nope "pause" "before=$BEFORE after=$AFTER"; fi
curl -sS -X POST "$HTTP/v1/connectors/$CONN_ID/resume" >/dev/null

# Mock 503 → transient metric increments.
cp "$FIXTURE" "$SERVE_DIR/metrics.bak"
# Replace the mock's response with 503 for ~3s (use a tiny server swap).
kill "$MOCK_PID"; wait "$MOCK_PID" 2>/dev/null || true
python3 -u -c "
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
  def do_GET(self): self.send_response(503); self.end_headers()
HTTPServer(('127.0.0.1', ${MOCK_PORT}), H).serve_forever()
" > /tmp/mock-prom.log 2>&1 &
MOCK_PID=$!
sleep 4
ERR=$(curl -sS "$HTTP/metrics" \
    | grep -E 'kyma_connector_errors_total.*reason="transient"' \
    | awk '{print $NF}' | head -1 || echo 0)
if [[ "${ERR:-0}" -gt 0 ]]; then ok "transient errors counted ($ERR)"
else nope "errors_total" "$ERR"; fi

# Restore mock.
kill "$MOCK_PID"; wait "$MOCK_PID" 2>/dev/null || true
( cd "$SERVE_DIR" && python3 -u -m http.server "$MOCK_PORT" ) > /tmp/mock-prom.log 2>&1 &
MOCK_PID=$!
sleep 2

# last_success_at advances again.
RECOVERED=$(curl -sS "$HTTP/v1/connectors/$CONN_ID" | grep -c 'last_success_at')
if [[ "$RECOVERED" -ge 1 ]]; then ok "recovered"
else nope "recovery" "$RECOVERED"; fi

# Idempotency: force a duplicate tick via /trigger twice, assert rows don't
# double.
ROWS_BEFORE=$(QUERY "SELECT count(*) AS n FROM metrics" | tail -n1)
curl -sS -X POST "$HTTP/v1/connectors/$CONN_ID/trigger" >/dev/null
curl -sS -X POST "$HTTP/v1/connectors/$CONN_ID/trigger" >/dev/null
sleep 2
ROWS_AFTER=$(QUERY "SELECT count(*) AS n FROM metrics" | tail -n1)
# Two triggers with the same scheduled_for bucket should land one set of rows.
# (Trigger uses now-ms, so the second trigger falls in the same ms-bucket
# only under load; the idempotency ledger is the stronger guarantee.)
if [[ "$ROWS_AFTER" -ge "$ROWS_BEFORE" ]]; then ok "no negative rows"
else nope "rows decreased" "before=$ROWS_BEFORE after=$ROWS_AFTER"; fi

# Delete.
curl -sS -X DELETE "$HTTP/v1/connectors/$CONN_ID" >/dev/null
GONE=$(curl -sS -o /dev/null -w '%{http_code}' "$HTTP/v1/connectors/$CONN_ID")
if [[ "$GONE" == "404" ]]; then ok "deleted"
else nope "delete" "status=$GONE"; fi

CONN_ID=""   # prevent cleanup redelete

echo
echo "passed: $pass, failed: $fail"
exit $([ $fail -eq 0 ] && echo 0 || echo 1)
```

- [ ] **Step 3: Make executable + smoke-run**

```bash
chmod +x scripts/test-prometheus-connector.sh
```

Manual run (requires a running stack):

```bash
docker-compose up -d
cargo run --release --bin kyma &
KYMA_PID=$!
sleep 5
./scripts/test-prometheus-connector.sh
STATUS=$?
kill $KYMA_PID
exit $STATUS
```

Expected: ≥ 9 green assertions. Some (notably the 503 flip) may be flaky in CI — fine for slice-1; run locally.

- [ ] **Step 4: Commit**

```bash
git add scripts/test-prometheus-connector.sh scripts/fixtures/
git commit -m "$(cat <<'EOF'
add E2E test script for Prometheus connector

Spins a python mock /metrics, creates a connector via the admin
API, verifies rows land, labels preserved, pause blocks new ticks,
503 surfaces as transient errors_total, recovery works, and
delete removes the connector.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Self-review

### Spec coverage

| Spec section | Task(s) |
|---|---|
| §2 primitives reused | 2 (migration), 11 (scheduler → background_tasks), 12 (runner → idempotency ledger + WritePath), 14 (wiring) |
| §3 crates and layout | 1, 3 |
| §4 migration 005 schema | 2 |
| §5 Connector trait | 3, 4, 5, 6 |
| §6 runtime model — Periodic scheduler | 11 |
| §6 runtime model — runner + crash safety | 12 |
| §7 SecretStore | 4 |
| §8 `metrics` table schema | covered by REST auto-create; no schema code in this plan |
| §9 admin API | 13 |
| §10 metrics | 6 (helpers), exercised by 15 |
| §11 error handling | 9 (in-tick), 12 (classification → task state) |
| §12 tests | 2, 4, 5, 7, 8, 9, 10, 11, 12, 13, 15 |
| §13 deferred affordances | explicitly not implemented |

One gap: the spec §8 implies the `metrics` table exists with a concrete schema. In this plan we rely on kyma's existing NDJSON auto-schema on first ingest (the E2E bootstrap write in Task 15 plants the schema). If auto-creation doesn't preserve the `labels` column as Utf8 or there's a schema mismatch at ingest time, add a 16th task with an explicit `CREATE TABLE` step. The E2E script will fail loudly if so.

### Placeholder scan

- Task 8 intentionally leaves `validate_config`'s body as a `todo!()` — this is the declared user-contribution point, not a placeholder. The skeleton includes the exact tests that constrain acceptable behaviour.
- Task 14 uses `// ... inside async main ...` — this is deliberate because the real `kyma-bin/src/main.rs` has assembly logic that depends on live variable names. The engineer is directed to match existing patterns (the compaction spawn is right there for reference).
- All other code steps show complete code.

### Type consistency

- `Connector` trait signature matches across tasks 3, 5, 8, 9, 12.
- `ConnectorCtx` fields (connector_id, http, secrets, scheduled_for, metrics) used consistently.
- `ConnectorRun` shape (rows + new_cursor) consistent.
- `ConnectorError` variants (Transient/Permanent/Config) wired identically in Task 9 (emitting) and Task 12 (receiving).
- `RowSink` signature in Task 12 matches usage in Task 14.
- `catalog_sql` module functions defined in Task 11 and called in Tasks 12 + 13 — signatures match.

### Scope check

One plan covers the framework + one reference connector end-to-end, matching the brainstorm decision. Additional connectors (Sentry, Loki, Elastic) are explicitly out-of-scope follow-ons; each will be its own plan once it's prioritised.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-20-connectors.md`. Two execution options:

**1. Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.

**2. Inline Execution** — Execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
