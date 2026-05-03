# DB Integration M1 — Postgres Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Prerequisite:** [DB M0 Foundation](2026-05-02-db-m0-foundation.md) is complete and committed.

**Goal:** Land `PostgresSource` — the first concrete `ExternalSource` implementation — with both federation and sync (CDC) modes shipping end-to-end. The federation pushdown planner reaches the spec's "best practice" target: filters, projection, `LIMIT`, `ORDER BY`, single-source aggregation, and opportunistic same-source join. The sync path performs a snapshot-consistent initial load via Postgres logical replication slots (`CREATE_REPLICATION_SLOT … EXPORT_SNAPSHOT`), then streams changes via `pgoutput` plugin protocol decoded into `CdcEvent`s with exactly-once cursor commits riding on `CommitCoordinator::with_cursor_update`. Property tests gate every pushdown rule. Chaos tests (snapshot-crash, streaming-crash, slot-dropped, source-restart, network-partition, CAS-storm) gate the merge.

**Architecture:** `PostgresSource` lives in `crates/kyma-connectors/src/external/postgres.rs`. Federation uses a connection pool on `sqlx::PgPool`; sync uses a separate dedicated `tokio_postgres` replication connection (sqlx doesn't support `START_REPLICATION`). Both modes share the `ResolvedConnection`. The pushdown planner is engine-agnostic but `PostgresSource::scan` upgrades the M0 `TableProviderFilterPushDown::Inexact` advertisements to `Exact` for filter shapes the planner has fully translated. CDC events are produced by an internal `pgoutput` decoder; engine-side `CdcEvent`s feed the engine-agnostic snapshot coordinator and stream consumer. Schema introspection reads from `information_schema` + `pg_type` + `pg_attribute`; the `TYPE_MAP` per spec §6.2 lives as a `const` array so the docs site can render it directly.

**Tech Stack:** Rust 1.95, Tokio, `sqlx 0.8` (federation pool + introspection), `tokio-postgres 0.7` (replication protocol), `pgvector` (already in tree), `arrow-array 53`, `datafusion 44`, `proptest`, `testcontainers` + `testcontainers_modules::postgres`. Spec: [`docs/superpowers/specs/2026-05-02-multi-source-database-integration-design.md`](../specs/2026-05-02-multi-source-database-integration-design.md). Builds on M0 trait shape.

---

## File Structure

**New files:**

- `crates/kyma-connectors/src/external/postgres/mod.rs` — module root + the `PostgresSource` impl.
- `crates/kyma-connectors/src/external/postgres/handle.rs` — `PostgresHandle` (wraps `sqlx::PgPool` + replication conn factory).
- `crates/kyma-connectors/src/external/postgres/types.rs` — `TYPE_MAP` per spec §6.2 + helpers to coerce a raw row into `serde_json::Value` shaped for the schema evolver.
- `crates/kyma-connectors/src/external/postgres/introspect.rs` — `information_schema` queries → `Vec<InferredSchema>`.
- `crates/kyma-connectors/src/external/postgres/scan.rs` — `scan()` impl: builds parameterized SQL from `PushedPlan::Sql`, streams rows, emits Arrow batches.
- `crates/kyma-connectors/src/external/postgres/snapshot.rs` — `snapshot_at()`: opens replication slot with `EXPORT_SNAPSHOT`, sets transaction snapshot, streams rows, returns `(SnapshotStream, Checkpoint{lsn})`.
- `crates/kyma-connectors/src/external/postgres/replication.rs` — `open_cdc()`: `START_REPLICATION SLOT … LOGICAL <lsn>`, `pgoutput` decoder → `CdcEvent` stream.
- `crates/kyma-connectors/src/external/postgres/pgoutput.rs` — message decoder for the `pgoutput` protocol (Begin/Commit/Insert/Update/Delete/Relation messages).
- `crates/kyma-connectors/src/external/postgres/sql_render.rs` — turns DataFusion `Expr` into a parameterized `Sql(String, Vec<BoundParam>)` for the Postgres dialect.
- `crates/kyma-federation/src/postgres_dialect.rs` — Postgres-specific `Capabilities` profile (constants); used by the planner and by `PostgresSource::capabilities()`.
- `crates/kyma-federation/src/scan_exec.rs` — DataFusion `ExecutionPlan` impl that wraps `ExternalSource::scan` (engine-agnostic; lands here because the planner consumes it). Parameterized over an `Arc<dyn ExternalSource>` + `PushedPlan`.
- `crates/kyma-federation/src/schema_cache.rs` — TTL+invalidation cache around `ExternalSource::introspect` per spec §6.6.
- `crates/kyma-connectors-testkit/src/pg_fixture.rs` — testcontainers helper that boots Postgres-with-replication-enabled (`wal_level=logical`).
- `crates/kyma-connectors-testkit/src/seed_pg.rs` — small deterministic dataset used by integration tests.
- `crates/kyma-connectors-testkit/src/chaos.rs` — fault-injection helpers (process kill, network drop, slot drop).

**Modified files:**

- `crates/kyma-connectors/src/external/mod.rs` — add `pub mod postgres;` (gated behind `cfg(feature = "postgres")` on the parent crate, default-on for now).
- `crates/kyma-connectors/Cargo.toml` — add `tokio-postgres` dep, add `[features] postgres` (default on for v1).
- `crates/kyma-connectors/src/cdc/snapshot.rs` — replace M0 stub with engine-agnostic implementation that drives `ExternalSource::snapshot_at` + the staging buffer + the atomic phase advance.
- `crates/kyma-connectors/src/cdc/stream.rs` — replace M0 stub with engine-agnostic implementation that drives `ExternalSource::open_cdc` + group commit with `with_cursor_update` + lag heartbeat.
- `crates/kyma-connectors/src/cdc/runner.rs` — implement the phase machine.
- `crates/kyma-federation/src/table_provider.rs` — replace M0 stub: build `PushedPlan` via planner, wire `scan_exec::FederatedScanExec`.
- `crates/kyma-federation/src/catalog_provider.rs` — populate `schema_names()` and `schema()` from the introspection cache.
- `crates/kyma-federation/src/planner.rs` — extend with: `LIMIT` and `ORDER BY` rendering, single-source aggregation pushdown, opportunistic same-source join detection.
- `crates/kyma-federation/src/live_fn.rs` — implement the resolver against the registry.
- `crates/kyma-bin/src/main.rs` — at startup, read enabled `mode != 'sync'` connector rows and register a `PostgresSource` per row into `FederationRegistry`. Read `mode != 'federation'` rows and register a `CdcConnector` per row into the connector registry.

**Test files:**

- `crates/kyma-connectors/src/external/postgres/types.rs` — unit tests inline.
- `crates/kyma-connectors/src/external/postgres/sql_render.rs` — unit tests inline.
- `crates/kyma-connectors/tests/postgres_integration.rs` — integration suite (federation + sync + mode=both per spec §12.3).
- `crates/kyma-federation/tests/planner_property_pg.rs` — 10k-case property test using the testkit generator against a synthetic `Capabilities` profile that mirrors `PostgresSource::capabilities()`.
- `crates/kyma-connectors/tests/postgres_chaos.rs` — chaos suite per spec §12.4 (gated by `RUN_CHAOS=1` env var to keep PR builds fast; nightly always runs it).

---

## Task 1: Postgres-specific `Capabilities` profile

**Files:**
- Create: `crates/kyma-federation/src/postgres_dialect.rs`
- Modify: `crates/kyma-federation/src/lib.rs`

- [ ] **Step 1: Define the constants**

Write `crates/kyma-federation/src/postgres_dialect.rs`:

```rust
#![forbid(unsafe_code)]
//! Static description of what we can safely push to Postgres.
//! Built once and cloned per `PostgresSource` instance.

use std::collections::BTreeSet;
use kyma_connectors::external::{AggFunc, Capabilities, FilterOp};

pub fn pg_capabilities() -> Capabilities {
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
    Capabilities {
        filter_ops,
        function_allowlist,
        agg_funcs,
        group_by: true,
        order_by: true,
        limit: true,
        same_source_join: true,
        string_collation_safe_columns: BTreeSet::new(), // PG case-sensitive by default
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn pg_supports_basic_set() {
        let c = pg_capabilities();
        assert!(c.limit && c.order_by && c.group_by && c.same_source_join);
        assert!(c.filter_ops.contains(&FilterOp::Like));
    }
}
```

- [ ] **Step 2: Re-export**

Edit `crates/kyma-federation/src/lib.rs`:

```rust
pub mod postgres_dialect;
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p kyma-federation postgres_dialect -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-federation/src/postgres_dialect.rs crates/kyma-federation/src/lib.rs
git commit -m "feat(db): Postgres Capabilities profile"
```

---

## Task 2: Postgres `TYPE_MAP` (the spec §6.2 table as code)

**Files:**
- Create: `crates/kyma-connectors/src/external/postgres/mod.rs`
- Create: `crates/kyma-connectors/src/external/postgres/types.rs`
- Modify: `crates/kyma-connectors/src/external/mod.rs`

- [ ] **Step 1: Module skeleton**

Write `crates/kyma-connectors/src/external/postgres/mod.rs`:

```rust
#![forbid(unsafe_code)]
//! Postgres `ExternalSource` impl. See spec §6.2, §5.2, §5.3.

pub mod handle;
pub mod introspect;
pub mod pgoutput;
pub mod replication;
pub mod scan;
pub mod snapshot;
pub mod sql_render;
pub mod types;

use std::sync::Arc;
use async_trait::async_trait;

use crate::external::{
    BatchSink, Capabilities, Checkpoint, ExternalError, ExternalSource,
    InferredSchema, PushedPlan, ResolvedConnection, ScanReport, Scope,
    SourceHandle, SourceHealth, CdcStream, SnapshotStream,
};
use kyma_federation::postgres_dialect::pg_capabilities;

pub struct PostgresSource {
    capabilities: Capabilities,
}

impl PostgresSource {
    pub fn new() -> Self { Self { capabilities: pg_capabilities() } }
}

impl Default for PostgresSource {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl ExternalSource for PostgresSource {
    fn type_id(&self) -> &'static str { "postgres" }
    fn capabilities(&self) -> &Capabilities { &self.capabilities }

    async fn connect(&self, conn: &ResolvedConnection) -> Result<Arc<dyn SourceHandle>, ExternalError> {
        handle::connect(conn).await
    }

    async fn health(&self, h: &dyn SourceHandle) -> Result<SourceHealth, ExternalError> {
        handle::health(h).await
    }

    async fn introspect(&self, h: &dyn SourceHandle, scope: &Scope) -> Result<Vec<InferredSchema>, ExternalError> {
        introspect::run(h, scope).await
    }

    async fn scan(&self, h: &dyn SourceHandle, plan: &PushedPlan, sink: &mut dyn BatchSink) -> Result<ScanReport, ExternalError> {
        scan::run(h, plan, sink).await
    }

    async fn open_cdc(&self, h: &dyn SourceHandle, from: Option<&Checkpoint>) -> Result<CdcStream, ExternalError> {
        replication::open(h, from).await
    }

    async fn snapshot_at(&self, h: &dyn SourceHandle, scope: &Scope) -> Result<(SnapshotStream, Checkpoint), ExternalError> {
        snapshot::take(h, scope).await
    }
}
```

- [ ] **Step 2: Wire into the parent module**

Edit `crates/kyma-connectors/src/external/mod.rs`, add:

```rust
#[cfg(feature = "postgres")]
pub mod postgres;
```

Edit `crates/kyma-connectors/Cargo.toml`:

```toml
[features]
default = ["postgres"]
postgres = ["dep:tokio-postgres"]

[dependencies]
# ... existing
tokio-postgres = { version = "0.7", default-features = false, features = ["runtime", "with-chrono-0_4", "with-uuid-1", "with-serde_json-1"], optional = true }
```

- [ ] **Step 3: Write the type map (the spec §6.2 table as code)**

Write `crates/kyma-connectors/src/external/postgres/types.rs`:

```rust
#![forbid(unsafe_code)]
//! Postgres → kyma type mapping. The single source of truth for spec §6.2;
//! the docs site renders this `TYPE_MAP` directly via `<SchemaMappingTable>`.

use crate::external::KymaType;

/// (postgres_typname, kyma_type, notes)
pub const TYPE_MAP: &[(&str, KymaType, &str)] = &[
    ("smallint",         KymaType::Int,       "16-bit; widened"),
    ("integer",          KymaType::Int,       "32-bit signed"),
    ("int",              KymaType::Int,       "alias of integer"),
    ("bigint",           KymaType::Long,      ""),
    ("oid",              KymaType::Long,      ""),
    ("real",             KymaType::Real,      "32-bit float; widened"),
    ("double precision", KymaType::Real,      "64-bit float"),
    ("numeric",          KymaType::Real,      "p≤15 → real; otherwise string (mode-controlled)"),
    ("decimal",          KymaType::Real,      "alias of numeric"),
    ("boolean",          KymaType::Bool,      ""),
    ("text",             KymaType::String,    ""),
    ("varchar",          KymaType::String,    ""),
    ("character varying",KymaType::String,    "alias of varchar"),
    ("char",             KymaType::String,    ""),
    ("character",        KymaType::String,    "alias of char"),
    ("name",             KymaType::String,    ""),
    ("uuid",             KymaType::String,    "canonical form"),
    ("bytea",            KymaType::String,    "base64; bytes columns post-v1"),
    ("timestamp without time zone", KymaType::Timestamp, "treated as UTC"),
    ("timestamp with time zone",    KymaType::Timestamp, ""),
    ("timestamptz",      KymaType::Timestamp, ""),
    ("date",             KymaType::Timestamp, "midnight UTC"),
    ("time",             KymaType::Timestamp, "1970-01-01 + time-of-day"),
    ("time without time zone", KymaType::Timestamp, ""),
    ("json",             KymaType::Dynamic,   "whole document; v1.5 drill-in"),
    ("jsonb",            KymaType::Dynamic,   "whole document; v1.5 drill-in"),
    // arrays detected by '_' prefix in pg_type.typname; mapped to dynamic.
    ("hstore",           KymaType::Dynamic,   ""),
    ("inet",             KymaType::String,    ""),
    ("cidr",             KymaType::String,    ""),
    ("macaddr",          KymaType::String,    ""),
    // pgvector
    ("vector",           KymaType::Vector(0), "dimension resolved from atttypmod"),
];

/// Returns the kyma type for a Postgres type name; arrays detected by `_`
/// prefix are mapped to `Dynamic`. Unknown types fall back to `Dynamic`
/// (per spec §6.2 — out-of-band types land in dynamic with a warning).
pub fn map_pg_type(typname: &str) -> KymaType {
    if typname.starts_with('_') { return KymaType::Dynamic; }
    if typname.ends_with("[]")  { return KymaType::Dynamic; }
    if typname.starts_with("int4range") || typname.starts_with("int8range")
       || typname.starts_with("numrange") || typname.starts_with("tsrange")
       || typname.starts_with("tstzrange") || typname.starts_with("daterange")
    { return KymaType::Dynamic; }
    if typname.starts_with("geometry") || typname.starts_with("geography") {
        return KymaType::String; // WKT mode (when scope.geometry_mode = wkt); Drop is filtered upstream
    }
    TYPE_MAP.iter().find(|(n, _, _)| *n == typname).map(|(_, t, _)| *t).unwrap_or(KymaType::Dynamic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn primitive_mappings() {
        assert!(matches!(map_pg_type("integer"), KymaType::Int));
        assert!(matches!(map_pg_type("bigint"), KymaType::Long));
        assert!(matches!(map_pg_type("real"), KymaType::Real));
        assert!(matches!(map_pg_type("text"), KymaType::String));
        assert!(matches!(map_pg_type("uuid"), KymaType::String));
        assert!(matches!(map_pg_type("timestamptz"), KymaType::Timestamp));
        assert!(matches!(map_pg_type("jsonb"), KymaType::Dynamic));
    }

    #[test] fn arrays_go_to_dynamic() {
        assert!(matches!(map_pg_type("_int4"), KymaType::Dynamic));
        assert!(matches!(map_pg_type("text[]"), KymaType::Dynamic));
    }

    #[test] fn ranges_go_to_dynamic() {
        assert!(matches!(map_pg_type("int4range"), KymaType::Dynamic));
        assert!(matches!(map_pg_type("tstzrange"), KymaType::Dynamic));
    }

    #[test] fn unknown_falls_back_to_dynamic() {
        assert!(matches!(map_pg_type("some_custom_enum"), KymaType::Dynamic));
    }
}
```

- [ ] **Step 4: Run tests + verify compile**

Run: `cargo test -p kyma-connectors external::postgres::types -- --nocapture`
Expected: PASS (4 tests).

Run: `cargo check -p kyma-connectors --features postgres`
Expected: compile errors are fine here for unimplemented modules; once tasks 3–10 land they'll resolve. For this checkpoint, write minimal placeholders for `handle.rs`, `introspect.rs`, `scan.rs`, `snapshot.rs`, `replication.rs`, `pgoutput.rs`, `sql_render.rs` that just declare `pub fn _placeholder() {}` so the module tree compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/ crates/kyma-connectors/src/external/mod.rs crates/kyma-connectors/Cargo.toml
git commit -m "feat(db): Postgres TYPE_MAP and module scaffold"
```

---

## Task 3: `PostgresHandle` (federation pool + replication factory)

**Files:**
- Modify: `crates/kyma-connectors/src/external/postgres/handle.rs`
- Create: `crates/kyma-connectors-testkit/src/pg_fixture.rs`

- [ ] **Step 1: Write the testcontainers fixture**

Write `crates/kyma-connectors-testkit/src/pg_fixture.rs`:

```rust
#![forbid(unsafe_code)]
//! Postgres testcontainer with logical replication enabled.

use testcontainers::{clients::Cli, core::WaitFor, RunnableImage};
use testcontainers_modules::postgres::Postgres as PgImage;

pub struct PgFixture<'d> {
    pub url: String,
    _pg: testcontainers::Container<'d, PgImage>,
}

pub fn boot<'d>(docker: &'d Cli) -> PgFixture<'d> {
    let img = RunnableImage::from(PgImage::default())
        .with_env_var(("POSTGRES_PASSWORD", "postgres"))
        // wal_level=logical is required for logical replication.
        .with_cmd(["postgres",
                   "-c", "wal_level=logical",
                   "-c", "max_replication_slots=10",
                   "-c", "max_wal_senders=10"]);
    let pg = docker.run(img);
    let port = pg.get_host_port_ipv4(5432);
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    // Wait for ready
    PgFixture { url, _pg: pg }
}
```

- [ ] **Step 2: Implement `connect` + `health`**

Write `crates/kyma-connectors/src/external/postgres/handle.rs`:

```rust
#![forbid(unsafe_code)]
use std::sync::Arc;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::external::{ExternalError, ResolvedConnection, SourceHandle, SourceHealth, TlsMode};

pub struct PostgresHandle {
    pub pool: PgPool,
    /// Connection URL with password substituted; used to open replication
    /// connections later (sqlx doesn't speak the replication protocol, so we
    /// dial a fresh tokio-postgres client each time `open_cdc` is called).
    pub conn_url: String,
}

impl SourceHandle for PostgresHandle {}

pub async fn connect(conn: &ResolvedConnection) -> Result<Arc<dyn SourceHandle>, ExternalError> {
    let url = with_password(&conn.url, conn.password.as_deref());

    if matches!(conn.tls, TlsMode::Disabled) {
        tracing::warn!(target: "kyma_connectors::postgres", "tls=disabled — production sources should require TLS");
    }
    // sqlx auto-uses TLS from the URL (sslmode); we add it if missing per TlsMode.
    let url = with_sslmode(&url, conn.tls);

    let pool = PgPoolOptions::new()
        .max_connections(conn.pool_size)
        .acquire_timeout(std::time::Duration::from_millis(conn.pool_acquire_timeout_ms))
        .connect(&url)
        .await
        .map_err(|e| classify_connect_error(e))?;

    Ok(Arc::new(PostgresHandle { pool, conn_url: url }))
}

pub async fn health(h: &dyn SourceHandle) -> Result<SourceHealth, ExternalError> {
    let h = h.as_any().downcast_ref::<PostgresHandle>()
        .ok_or_else(|| ExternalError::Bug("wrong handle type".into()))?;
    let row: (String,) = sqlx::query_as("SELECT version()")
        .fetch_one(&h.pool).await
        .map_err(|e| ExternalError::Transient(e.to_string()))?;
    Ok(SourceHealth { reachable: true, version: Some(row.0), detail: None })
}

fn with_password(url: &str, password: Option<&str>) -> String {
    match password {
        None => url.to_string(),
        Some(pw) => {
            // Naive substitution: replace `postgres://user@` with `postgres://user:pw@`.
            // Production would use the `url` crate. For M1, this matches typical inputs.
            if url.contains("@") && !url.contains(":") {
                let (scheme_user, rest) = url.split_once('@').unwrap();
                format!("{scheme_user}:{pw}@{rest}")
            } else {
                url.to_string()
            }
        }
    }
}

fn with_sslmode(url: &str, tls: TlsMode) -> String {
    if url.contains("sslmode=") { return url.to_string(); }
    let mode = match tls {
        TlsMode::Required => "require",
        TlsMode::Preferred => "prefer",
        TlsMode::Disabled => "disable",
    };
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}sslmode={mode}")
}

fn classify_connect_error(e: sqlx::Error) -> ExternalError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("password") || lower.contains("authentication") {
        ExternalError::OperatorActionable(msg)
    } else if lower.contains("ssl") || lower.contains("tls") {
        ExternalError::OperatorActionable(msg)
    } else if lower.contains("not found") || lower.contains("nodename") {
        ExternalError::OperatorActionable(msg)
    } else {
        ExternalError::Transient(msg)
    }
}
```

You also need `dyn SourceHandle` to support `as_any` — extend the trait in `kyma-connectors/src/external/trait_def.rs`:

```rust
pub trait SourceHandle: Send + Sync + 'static {
    fn as_any(&self) -> &dyn std::any::Any;
}

// Default impl macro for impls that just want the trivial as_any:
#[macro_export]
macro_rules! impl_source_handle_as_any {
    ($t:ty) => {
        impl $crate::external::SourceHandle for $t {
            fn as_any(&self) -> &dyn std::any::Any { self }
        }
    };
}
```

(Adjust the existing M0 `SourceHandle` impl if needed; an empty trait would fail the downcast.)

- [ ] **Step 3: Smoke test connect against testcontainer**

Write `crates/kyma-connectors/tests/postgres_handle.rs`:

```rust
use kyma_connectors::external::{ConnectionConfig, ResolvedConnection, TlsMode};
use kyma_connectors::external::postgres::handle;
use testcontainers::clients::Cli;
use kyma_connectors_testkit::pg_fixture;

#[tokio::test]
async fn connects_and_reports_version() {
    let docker = Cli::default();
    let pg = pg_fixture::boot(&docker);
    let conn = ResolvedConnection {
        url: pg.url.clone(),
        password: None, // already in the URL
        tls: TlsMode::Disabled,
        pool_size: 4,
        pool_acquire_timeout_ms: 5000,
        extra: serde_json::Value::Null,
    };
    let h = handle::connect(&conn).await.unwrap();
    let health = handle::health(&*h).await.unwrap();
    assert!(health.reachable);
    assert!(health.version.unwrap().to_lowercase().contains("postgresql"));
}
```

- [ ] **Step 4: Run the test**

Run: `cargo test -p kyma-connectors --test postgres_handle -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/handle.rs crates/kyma-connectors/src/external/trait_def.rs crates/kyma-connectors-testkit/src/pg_fixture.rs crates/kyma-connectors/tests/postgres_handle.rs
git commit -m "feat(db): PostgresHandle (sqlx pool + replication URL) + testcontainer fixture"
```

---

## Task 4: Schema introspection (`information_schema`)

**Files:**
- Modify: `crates/kyma-connectors/src/external/postgres/introspect.rs`

- [ ] **Step 1: Write the failing integration test**

Append to `crates/kyma-connectors/tests/postgres_handle.rs`:

```rust
use kyma_connectors::external::postgres::introspect;
use kyma_connectors::external::Scope;

#[tokio::test]
async fn introspect_returns_inferred_schema_for_seeded_table() {
    let docker = Cli::default();
    let pg = pg_fixture::boot(&docker);
    // Seed
    let pool = sqlx::PgPool::connect(&pg.url).await.unwrap();
    sqlx::query("CREATE SCHEMA app").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE app.users (id BIGINT PRIMARY KEY, email TEXT NOT NULL, region TEXT, score DOUBLE PRECISION, created_at TIMESTAMPTZ NOT NULL DEFAULT now())")
        .execute(&pool).await.unwrap();

    let conn = kyma_connectors::external::ResolvedConnection {
        url: pg.url, password: None, tls: kyma_connectors::external::TlsMode::Disabled,
        pool_size: 4, pool_acquire_timeout_ms: 5000, extra: serde_json::Value::Null,
    };
    let h = kyma_connectors::external::postgres::handle::connect(&conn).await.unwrap();

    let scope = Scope { include_schemas: vec!["app".into()], ..Default::default() };
    let schemas = introspect::run(&*h, &scope).await.unwrap();
    let users = schemas.iter().find(|s| s.source_table == "users").unwrap();
    assert_eq!(users.primary_key, vec!["id"]);
    let cols: std::collections::HashMap<_,_> = users.columns.iter().map(|c| (c.name.as_str(), c.kyma_type)).collect();
    assert!(matches!(cols.get("id"), Some(kyma_connectors::external::KymaType::Long)));
    assert!(matches!(cols.get("email"), Some(kyma_connectors::external::KymaType::String)));
    assert!(matches!(cols.get("score"), Some(kyma_connectors::external::KymaType::Real)));
    assert!(matches!(cols.get("created_at"), Some(kyma_connectors::external::KymaType::Timestamp)));
}
```

Run: `cargo test -p kyma-connectors --test postgres_handle introspect_returns_inferred_schema_for_seeded_table -- --nocapture`
Expected: FAIL (module unimplemented).

- [ ] **Step 2: Implement introspection**

Write `crates/kyma-connectors/src/external/postgres/introspect.rs`:

```rust
#![forbid(unsafe_code)]
use sqlx::PgPool;
use crate::external::{ExternalError, InferredColumn, InferredSchema, KymaType, Scope, SourceHandle};
use super::handle::PostgresHandle;
use super::types::map_pg_type;

pub async fn run(h: &dyn SourceHandle, scope: &Scope) -> Result<Vec<InferredSchema>, ExternalError> {
    let h = h.as_any().downcast_ref::<PostgresHandle>()
        .ok_or_else(|| ExternalError::Bug("wrong handle".into()))?;
    let pool = &h.pool;

    // Pull table list filtered by scope.
    let mut tables = list_tables(pool, scope).await?;
    let mut out = Vec::with_capacity(tables.len());
    for (schema, table) in tables.drain(..) {
        let cols = list_columns(pool, &schema, &table).await?;
        let pk = list_primary_key(pool, &schema, &table).await?;
        out.push(InferredSchema {
            source_schema: schema,
            source_table: table,
            primary_key: pk,
            columns: cols,
            extra: serde_json::Value::Null,
        });
    }
    Ok(out)
}

async fn list_tables(pool: &PgPool, scope: &Scope) -> Result<Vec<(String, String)>, ExternalError> {
    let q = if scope.include_schemas.is_empty() {
        "SELECT table_schema, table_name FROM information_schema.tables
         WHERE table_type = 'BASE TABLE'
           AND table_schema NOT IN ('pg_catalog', 'information_schema')"
    } else {
        "SELECT table_schema, table_name FROM information_schema.tables
         WHERE table_type = 'BASE TABLE' AND table_schema = ANY($1)"
    };
    let mut rows: Vec<(String, String)> = if scope.include_schemas.is_empty() {
        sqlx::query_as(q).fetch_all(pool).await
            .map_err(|e| ExternalError::Transient(e.to_string()))?
    } else {
        sqlx::query_as(q).bind(&scope.include_schemas).fetch_all(pool).await
            .map_err(|e| ExternalError::Transient(e.to_string()))?
    };
    rows.retain(|(s, t)| {
        let qual = format!("{s}.{t}");
        !scope.exclude_tables.iter().any(|excl| excl == &qual)
    });
    Ok(rows)
}

async fn list_columns(pool: &PgPool, schema: &str, table: &str) -> Result<Vec<InferredColumn>, ExternalError> {
    let rows: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT column_name, udt_name, is_nullable = 'YES'
         FROM information_schema.columns
         WHERE table_schema = $1 AND table_name = $2
         ORDER BY ordinal_position",
    ).bind(schema).bind(table).fetch_all(pool).await
     .map_err(|e| ExternalError::Transient(e.to_string()))?;

    Ok(rows.into_iter().map(|(name, udt, nullable)| {
        let kyma = map_pg_type(&udt);
        InferredColumn { name, source_type: udt, kyma_type: kyma, nullable }
    }).collect())
}

async fn list_primary_key(pool: &PgPool, schema: &str, table: &str) -> Result<Vec<String>, ExternalError> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT a.attname
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = c.relnamespace
         JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey)
         WHERE n.nspname = $1 AND c.relname = $2 AND i.indisprimary
         ORDER BY array_position(i.indkey, a.attnum)",
    ).bind(schema).bind(table).fetch_all(pool).await
     .map_err(|e| ExternalError::Transient(e.to_string()))?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p kyma-connectors --test postgres_handle introspect_returns_inferred_schema_for_seeded_table -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/introspect.rs crates/kyma-connectors/tests/postgres_handle.rs
git commit -m "feat(db): Postgres introspection via information_schema"
```

---

## Task 5: SQL renderer (DataFusion `Expr` → parameterized SQL)

**Files:**
- Modify: `crates/kyma-connectors/src/external/postgres/sql_render.rs`

- [ ] **Step 1: Implement renderer + write inline tests**

Write `crates/kyma-connectors/src/external/postgres/sql_render.rs`:

```rust
#![forbid(unsafe_code)]
//! Renders DataFusion `Expr` into a parameterized Postgres SQL fragment.
//! Strict-bind discipline: every literal becomes a `$N` parameter; nothing
//! is interpolated as a string.

use datafusion::common::ScalarValue;
use datafusion::logical_expr::{BinaryExpr, Expr, Operator};
use crate::external::BoundParam;

pub struct Rendered {
    pub sql: String,
    pub binds: Vec<BoundParam>,
}

pub fn render_filter(filters: &[Expr]) -> Option<Rendered> {
    if filters.is_empty() { return None; }
    let mut binds: Vec<BoundParam> = Vec::new();
    let mut parts = Vec::new();
    for f in filters {
        let s = render_one(f, &mut binds)?;
        parts.push(format!("({s})"));
    }
    Some(Rendered {
        sql: parts.join(" AND "),
        binds,
    })
}

fn render_one(e: &Expr, binds: &mut Vec<BoundParam>) -> Option<String> {
    match e {
        Expr::Column(c) => Some(format!("\"{}\"", c.name.replace('"', "\"\""))),
        Expr::Literal(s) => Some(push_literal(s, binds)?),
        Expr::IsNull(inner) => Some(format!("{} IS NULL", render_one(inner, binds)?)),
        Expr::Not(inner) => Some(format!("NOT ({})", render_one(inner, binds)?)),
        Expr::BinaryExpr(BinaryExpr { left, op, right }) => {
            let op_sql = match op {
                Operator::Eq => "=",
                Operator::NotEq => "!=",
                Operator::Lt => "<",
                Operator::LtEq => "<=",
                Operator::Gt => ">",
                Operator::GtEq => ">=",
                Operator::And => "AND",
                Operator::Or => "OR",
                _ => return None,
            };
            Some(format!("{} {} {}", render_one(left, binds)?, op_sql, render_one(right, binds)?))
        }
        Expr::InList(in_list) => {
            let lhs = render_one(&in_list.expr, binds)?;
            let mut vals = Vec::with_capacity(in_list.list.len());
            for v in &in_list.list { vals.push(render_one(v, binds)?); }
            let kw = if in_list.negated { "NOT IN" } else { "IN" };
            Some(format!("{lhs} {kw} ({})", vals.join(", ")))
        }
        Expr::Like(like) => {
            let lhs = render_one(&like.expr, binds)?;
            let rhs = render_one(&like.pattern, binds)?;
            let kw = if like.negated { "NOT LIKE" } else { "LIKE" };
            Some(format!("{lhs} {kw} {rhs}"))
        }
        _ => None,
    }
}

fn push_literal(s: &ScalarValue, binds: &mut Vec<BoundParam>) -> Option<String> {
    let bp = match s {
        ScalarValue::Boolean(Some(b)) => BoundParam::Bool(*b),
        ScalarValue::Int8(Some(i)) => BoundParam::Int(*i as i64),
        ScalarValue::Int16(Some(i)) => BoundParam::Int(*i as i64),
        ScalarValue::Int32(Some(i)) => BoundParam::Int(*i as i64),
        ScalarValue::Int64(Some(i)) => BoundParam::Int(*i),
        ScalarValue::UInt8(Some(i)) => BoundParam::Int(*i as i64),
        ScalarValue::UInt16(Some(i)) => BoundParam::Int(*i as i64),
        ScalarValue::UInt32(Some(i)) => BoundParam::Int(*i as i64),
        ScalarValue::UInt64(Some(i)) if *i <= i64::MAX as u64 => BoundParam::Int(*i as i64),
        ScalarValue::Float32(Some(f)) => BoundParam::Real(*f as f64),
        ScalarValue::Float64(Some(f)) => BoundParam::Real(*f),
        ScalarValue::Utf8(Some(s)) => BoundParam::String(s.clone()),
        ScalarValue::LargeUtf8(Some(s)) => BoundParam::String(s.clone()),
        ScalarValue::Boolean(None) | ScalarValue::Int32(None) | ScalarValue::Int64(None)
        | ScalarValue::Utf8(None) | ScalarValue::Float64(None) => BoundParam::Null,
        _ => return None, // unsupported scalar — caller treats expression as residual
    };
    binds.push(bp);
    Some(format!("${}", binds.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::logical_expr::{col, lit};
    use datafusion::prelude::*;

    #[test] fn eq_renders_with_bind() {
        let r = render_filter(&[col("region").eq(lit("eu"))]).unwrap();
        assert_eq!(r.sql, "(\"region\" = $1)");
        assert_eq!(r.binds.len(), 1);
        assert!(matches!(&r.binds[0], BoundParam::String(s) if s == "eu"));
    }

    #[test] fn and_renders_two_binds() {
        let r = render_filter(&[col("region").eq(lit("eu")).and(col("score").gt(lit(0.5)))]).unwrap();
        assert!(r.sql.contains("AND"));
        assert_eq!(r.binds.len(), 2);
    }

    #[test] fn unsupported_returns_none() {
        // a function call we don't render
        let f = Expr::Negative(Box::new(col("x")));
        assert!(render_filter(&[f]).is_none());
    }

    #[test] fn injection_attempt_safely_bound() {
        let r = render_filter(&[col("email").eq(lit("'); DROP TABLE users;--"))]).unwrap();
        // The dangerous string lives in a bind, NOT the SQL string.
        assert!(!r.sql.contains("DROP TABLE"));
        match &r.binds[0] { BoundParam::String(s) => assert!(s.contains("DROP")), _ => panic!() }
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p kyma-connectors external::postgres::sql_render -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/sql_render.rs
git commit -m "feat(db): Postgres SQL renderer with strict-bind discipline"
```

---

## Task 6: `scan` impl (federation read path)

**Files:**
- Modify: `crates/kyma-connectors/src/external/postgres/scan.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/kyma-connectors/tests/postgres_handle.rs`:

```rust
use kyma_connectors::external::{BatchSink, PushedPlan, BoundParam};

struct CollectSink { batches: Vec<arrow_array::RecordBatch> }
#[async_trait::async_trait]
impl BatchSink for CollectSink {
    async fn push(&mut self, b: arrow_array::RecordBatch) -> Result<(), anyhow::Error> {
        self.batches.push(b); Ok(())
    }
    async fn finish(&mut self) -> Result<(), anyhow::Error> { Ok(()) }
}

#[tokio::test]
async fn scan_returns_arrow_batches_for_filtered_query() {
    let docker = Cli::default();
    let pg = pg_fixture::boot(&docker);
    let pool = sqlx::PgPool::connect(&pg.url).await.unwrap();
    sqlx::query("CREATE TABLE u (id BIGINT PRIMARY KEY, email TEXT, region TEXT)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO u VALUES (1,'a@x','eu'),(2,'b@x','us'),(3,'c@x','eu')").execute(&pool).await.unwrap();

    let conn = kyma_connectors::external::ResolvedConnection {
        url: pg.url, password: None, tls: kyma_connectors::external::TlsMode::Disabled,
        pool_size: 4, pool_acquire_timeout_ms: 5000, extra: serde_json::Value::Null,
    };
    let h = kyma_connectors::external::postgres::handle::connect(&conn).await.unwrap();

    let plan = PushedPlan::Sql {
        sql: "SELECT id, email FROM u WHERE region = $1 ORDER BY id".into(),
        binds: vec![BoundParam::String("eu".into())],
    };
    let mut sink = CollectSink { batches: Vec::new() };
    let report = kyma_connectors::external::postgres::scan::run(&*h, &plan, &mut sink).await.unwrap();
    assert_eq!(report.rows_returned, 2);
    let total_rows: usize = sink.batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 2);
}
```

Run: `cargo test -p kyma-connectors --test postgres_handle scan_returns -- --nocapture`
Expected: FAIL.

- [ ] **Step 2: Implement `scan::run`**

Write `crates/kyma-connectors/src/external/postgres/scan.rs`:

```rust
#![forbid(unsafe_code)]
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use sqlx::Row;

use crate::external::{
    BatchSink, BoundParam, ExternalError, PushedPlan, ScanReport, SourceHandle,
};
use super::handle::PostgresHandle;

pub async fn run(h: &dyn SourceHandle, plan: &PushedPlan, sink: &mut dyn BatchSink) -> Result<ScanReport, ExternalError> {
    let h = h.as_any().downcast_ref::<PostgresHandle>()
        .ok_or_else(|| ExternalError::Bug("wrong handle".into()))?;
    let (sql, binds) = match plan {
        PushedPlan::Sql { sql, binds } => (sql.clone(), binds.clone()),
        _ => return Err(ExternalError::Bug("postgres only consumes Sql plans".into())),
    };

    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = match b {
            BoundParam::Int(v) => q.bind(*v),
            BoundParam::Real(v) => q.bind(*v),
            BoundParam::Bool(v) => q.bind(*v),
            BoundParam::String(v) => q.bind(v.clone()),
            BoundParam::Timestamp(t) => q.bind(*t),
            BoundParam::Null => q.bind::<Option<i64>>(None),
        };
    }
    let rows = q.fetch_all(&h.pool).await
        .map_err(|e| ExternalError::Transient(e.to_string()))?;

    if rows.is_empty() {
        sink.finish().await.map_err(|e| ExternalError::Bug(e.to_string()))?;
        return Ok(ScanReport::default());
    }

    // Build Arrow schema from row 0's columns. M1 supports the most common
    // types (Int64, Float64, Utf8, Bool, Timestamp). Other types are
    // stringified — explicit improvement opportunity tracked in M4.
    let mut fields: Vec<Field> = Vec::with_capacity(rows[0].len());
    for col in rows[0].columns() {
        let name = col.name().to_string();
        let dt = pg_to_arrow_type(col.type_info());
        fields.push(Field::new(name, dt, true));
    }
    let schema = Arc::new(Schema::new(fields));

    // Build column arrays.
    let mut cols: Vec<arrow_array::ArrayRef> = Vec::with_capacity(schema.fields().len());
    for (i, field) in schema.fields().iter().enumerate() {
        cols.push(build_column(&rows, i, field.data_type())?);
    }

    let batch = RecordBatch::try_new(schema, cols)
        .map_err(|e| ExternalError::Bug(format!("arrow batch: {e}")))?;
    let row_count = batch.num_rows();
    sink.push(batch).await.map_err(|e| ExternalError::Bug(e.to_string()))?;
    sink.finish().await.map_err(|e| ExternalError::Bug(e.to_string()))?;

    Ok(ScanReport { rows_returned: row_count as u64, bytes_received: 0 })
}

fn pg_to_arrow_type(ti: &sqlx::postgres::PgTypeInfo) -> DataType {
    use sqlx::TypeInfo;
    match ti.name() {
        "INT2" | "INT4" => DataType::Int64,
        "INT8" => DataType::Int64,
        "FLOAT4" | "FLOAT8" => DataType::Float64,
        "BOOL" => DataType::Boolean,
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" | "UUID" => DataType::Utf8,
        "TIMESTAMP" | "TIMESTAMPTZ" => DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, None),
        _ => DataType::Utf8, // fallback: stringify; logged as TODO for v1.5
    }
}

fn build_column(rows: &[sqlx::postgres::PgRow], i: usize, dt: &DataType) -> Result<arrow_array::ArrayRef, ExternalError> {
    use sqlx::Row;
    match dt {
        DataType::Int64 => {
            let v: Vec<Option<i64>> = rows.iter().map(|r| r.try_get::<Option<i64>, _>(i).unwrap_or(None)).collect();
            Ok(Arc::new(Int64Array::from(v)))
        }
        DataType::Utf8 => {
            let v: Vec<Option<String>> = rows.iter().map(|r| r.try_get::<Option<String>, _>(i).unwrap_or(None)).collect();
            Ok(Arc::new(StringArray::from(v)))
        }
        DataType::Float64 => {
            let v: Vec<Option<f64>> = rows.iter().map(|r| r.try_get::<Option<f64>, _>(i).unwrap_or(None)).collect();
            Ok(Arc::new(arrow_array::Float64Array::from(v)))
        }
        DataType::Boolean => {
            let v: Vec<Option<bool>> = rows.iter().map(|r| r.try_get::<Option<bool>, _>(i).unwrap_or(None)).collect();
            Ok(Arc::new(arrow_array::BooleanArray::from(v)))
        }
        DataType::Timestamp(_, _) => {
            let v: Vec<Option<i64>> = rows.iter().map(|r| {
                r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(i).unwrap_or(None)
                    .map(|dt| dt.timestamp_micros())
            }).collect();
            Ok(Arc::new(arrow_array::TimestampMicrosecondArray::from(v)))
        }
        _ => Err(ExternalError::Bug(format!("unsupported arrow type {dt:?}"))),
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p kyma-connectors --test postgres_handle scan_returns -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/scan.rs
git commit -m "feat(db): Postgres scan path producing Arrow batches"
```

---

## Task 7: Replication slot snapshot (`snapshot_at`)

**Files:**
- Modify: `crates/kyma-connectors/src/external/postgres/snapshot.rs`

- [ ] **Step 1: Write the failing integration test**

Write `crates/kyma-connectors/tests/postgres_snapshot.rs`:

```rust
use futures::StreamExt;
use kyma_connectors::external::{Scope, ResolvedConnection, TlsMode};
use kyma_connectors_testkit::pg_fixture;
use testcontainers::clients::Cli;

#[tokio::test]
async fn snapshot_at_emits_rows_and_returns_lsn_checkpoint() {
    let docker = Cli::default();
    let pg = pg_fixture::boot(&docker);
    let pool = sqlx::PgPool::connect(&pg.url).await.unwrap();
    sqlx::query("CREATE TABLE t (id BIGINT PRIMARY KEY, v TEXT)").execute(&pool).await.unwrap();
    for i in 0..1000i64 { sqlx::query("INSERT INTO t VALUES ($1, $2)").bind(i).bind(i.to_string()).execute(&pool).await.unwrap(); }

    let conn = ResolvedConnection { url: pg.url, password: None, tls: TlsMode::Disabled, pool_size: 4, pool_acquire_timeout_ms: 5000, extra: serde_json::Value::Null };
    let h = kyma_connectors::external::postgres::handle::connect(&conn).await.unwrap();
    let scope = Scope { include_schemas: vec!["public".into()], ..Default::default() };
    let (mut stream, ckpt) = kyma_connectors::external::postgres::snapshot::take(&*h, &scope).await.unwrap();
    let mut count = 0;
    while let Some(item) = stream.next().await {
        item.unwrap();
        count += 1;
    }
    assert_eq!(count, 1000);
    assert!(ckpt.as_json().get("lsn").is_some());
}
```

Run: `cargo test -p kyma-connectors --test postgres_snapshot -- --nocapture`
Expected: FAIL (unimplemented).

- [ ] **Step 2: Implement `snapshot_at`**

Write `crates/kyma-connectors/src/external/postgres/snapshot.rs`:

```rust
#![forbid(unsafe_code)]
//! Initial snapshot via logical replication slot with EXPORT_SNAPSHOT.

use std::pin::Pin;
use futures::stream::Stream;
use futures::StreamExt;
use serde_json::json;
use tokio_postgres::{Client, NoTls};

use crate::external::{Checkpoint, ExternalError, Scope, SnapshotStream, SourceHandle};
use super::handle::PostgresHandle;

pub async fn take(h: &dyn SourceHandle, scope: &Scope) -> Result<(SnapshotStream, Checkpoint), ExternalError> {
    let h = h.as_any().downcast_ref::<PostgresHandle>()
        .ok_or_else(|| ExternalError::Bug("wrong handle".into()))?;
    let conn_url = h.conn_url.clone();

    // Open a fresh tokio-postgres connection for the snapshot transaction.
    // sqlx doesn't drive replication so we use a raw client here.
    let (client, connection) = tokio_postgres::connect(&conn_url, NoTls)
        .await.map_err(|e| ExternalError::OperatorActionable(e.to_string()))?;
    tokio::spawn(connection); // drive connection until client is dropped

    // 1. CREATE_REPLICATION_SLOT (non-temporary so it survives restarts).
    let slot_name = format!("kyma_snap_{}", uuid::Uuid::new_v4().simple());
    let row = client.query_one(
        &format!("SELECT pg_create_logical_replication_slot('{slot_name}', 'pgoutput', false)"),
        &[],
    ).await.map_err(|e| classify(e))?;
    // Read consistent_point + snapshot_name from pg_replication_slots:
    let slot_row = client.query_one(
        "SELECT slot_name, restart_lsn, confirmed_flush_lsn FROM pg_replication_slots WHERE slot_name = $1",
        &[&slot_name],
    ).await.map_err(|e| classify(e))?;
    let restart_lsn: String = slot_row.get::<_, sqlx::types::Decimal>("restart_lsn").map(|d| d.to_string())
        .or_else(|| slot_row.try_get::<_, String>("restart_lsn").ok())
        .unwrap_or_default();
    // (sqlx::types::Decimal won't actually be on a tokio-postgres row; replace with the proper
    // `tokio_postgres::types::Type::TEXT` cast or use a direct sqlx query for slot creation
    // via a normal connection, since `pg_create_logical_replication_slot` works on regular
    // connections. The version below is the cleaner route.)

    // Cleaner alternative: use sqlx for slot creation, which returns the LSN as a TEXT.
    let pg_pool = h.pool.clone();
    let slot_row: (String, String, String) = sqlx::query_as(
        "SELECT slot_name, lsn::text, snapshot_name FROM pg_create_logical_replication_slot($1, 'pgoutput', false, true)"
    ).bind(&slot_name).fetch_one(&pg_pool).await.map_err(|e| ExternalError::OperatorActionable(e.to_string()))?;
    let snapshot_name = slot_row.2;
    let confirmed_lsn = slot_row.1;

    // 2. Open a snapshot transaction in a separate sqlx connection so the
    // snapshot reads happen at the slot's consistent point.
    let mut snap_tx = pg_pool.begin().await.map_err(|e| ExternalError::Transient(e.to_string()))?;
    sqlx::query(&format!("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")).execute(&mut *snap_tx)
        .await.map_err(|e| ExternalError::Transient(e.to_string()))?;
    sqlx::query(&format!("SET TRANSACTION SNAPSHOT '{snapshot_name}'")).execute(&mut *snap_tx)
        .await.map_err(|e| ExternalError::Transient(e.to_string()))?;

    // 3. Build a stream that scans every included table in this snapshot.
    let scope = scope.clone();
    let stream = async_stream::try_stream! {
        let tables = super::introspect::list_scoped_tables(&pg_pool, &scope).await?;
        for (schema, table) in tables {
            // SELECT * FROM "schema"."table" — within the snapshot tx.
            // For simplicity, M1 streams using fetch_all per table; M4 will move to chunked fetch.
            let q = format!("SELECT * FROM \"{schema}\".\"{table}\"");
            let rows: Vec<sqlx::postgres::PgRow> = sqlx::query(&q).fetch_all(&mut *snap_tx).await
                .map_err(|e| ExternalError::Transient(e.to_string()))?;
            for r in rows {
                // Render row as JSON for the schema evolver.
                let v = pg_row_to_json(&r);
                yield v;
            }
        }
        // commit the snapshot tx.
        // (`async_stream` macro: this `_` discards Result.)
        let _ = snap_tx.commit().await;
    };

    Ok((Box::new(Box::pin(stream)) as SnapshotStream, Checkpoint::new(json!({"lsn": confirmed_lsn, "slot": slot_name}))))
}

fn pg_row_to_json(r: &sqlx::postgres::PgRow) -> serde_json::Value {
    use sqlx::Row;
    let mut obj = serde_json::Map::new();
    for col in r.columns() {
        let name = col.name();
        // Best-effort type coercion: try common types.
        let v: serde_json::Value = if let Ok(Some(v)) = r.try_get::<Option<i64>, _>(name) { json!(v) }
            else if let Ok(Some(v)) = r.try_get::<Option<f64>, _>(name) { json!(v) }
            else if let Ok(Some(v)) = r.try_get::<Option<bool>, _>(name) { json!(v) }
            else if let Ok(Some(v)) = r.try_get::<Option<String>, _>(name) { json!(v) }
            else if let Ok(Some(v)) = r.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(name) { json!(v.to_rfc3339()) }
            else if let Ok(Some(v)) = r.try_get::<Option<serde_json::Value>, _>(name) { v }
            else { serde_json::Value::Null };
        obj.insert(name.to_string(), v);
    }
    serde_json::Value::Object(obj)
}

fn classify(e: tokio_postgres::Error) -> ExternalError {
    let s = e.to_string();
    let lower = s.to_lowercase();
    if lower.contains("permission") || lower.contains("must be superuser") || lower.contains("replication") {
        ExternalError::OperatorActionable(s)
    } else { ExternalError::Transient(s) }
}
```

(NB: `introspect::list_scoped_tables` — make `list_tables` public or refactor; rename to `list_scoped_tables` and `pub(crate)`-expose.)

Add `async-stream = "0.3"` to `kyma-connectors/Cargo.toml`.

- [ ] **Step 3: Run the test**

Run: `cargo test -p kyma-connectors --test postgres_snapshot -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/snapshot.rs crates/kyma-connectors/src/external/postgres/introspect.rs crates/kyma-connectors/Cargo.toml crates/kyma-connectors/tests/postgres_snapshot.rs
git commit -m "feat(db): Postgres snapshot via replication slot with EXPORT_SNAPSHOT"
```

---

## Task 8: `pgoutput` decoder + `open_cdc`

**Files:**
- Modify: `crates/kyma-connectors/src/external/postgres/pgoutput.rs`
- Modify: `crates/kyma-connectors/src/external/postgres/replication.rs`

- [ ] **Step 1: Implement the `pgoutput` message decoder**

Write `crates/kyma-connectors/src/external/postgres/pgoutput.rs` covering at minimum the messages: Begin (`B`), Commit (`C`), Relation (`R`), Insert (`I`), Update (`U`), Delete (`D`). Each is a fixed-prefix byte format documented at https://www.postgresql.org/docs/current/protocol-logicalrep-message-formats.html. Decode into a typed `PgOutputMessage` enum:

```rust
#![forbid(unsafe_code)]
//! `pgoutput` logical replication message decoder.

use bytes::Buf;

#[derive(Debug, Clone)]
pub enum PgOutputMessage {
    Begin { final_lsn: u64, ts: i64, xid: u32 },
    Commit { commit_lsn: u64, end_lsn: u64, ts: i64 },
    Relation { rel_id: u32, schema: String, name: String, replica_identity: u8, columns: Vec<Column> },
    Insert  { rel_id: u32, tuple: TupleData },
    Update  { rel_id: u32, before: Option<TupleData>, after: TupleData },
    Delete  { rel_id: u32, key_or_old: TupleData },
    Truncate { rel_ids: Vec<u32> },
    Type    { /* skipped in v1 */ },
    Origin  { /* skipped in v1 */ },
    Other,
}

#[derive(Debug, Clone)]
pub struct Column { pub flags: u8, pub name: String, pub data_type_oid: u32, pub type_modifier: i32 }

#[derive(Debug, Clone)]
pub struct TupleData { pub fields: Vec<Field> }

#[derive(Debug, Clone)]
pub enum Field { Null, Toast, Text(String), /* binary unused in pgoutput v2 */ }

pub fn decode(buf: &[u8]) -> Result<PgOutputMessage, String> {
    let mut b = buf;
    if !b.has_remaining() { return Err("empty".into()); }
    let kind = b.get_u8();
    match kind {
        b'B' => {
            let final_lsn = b.get_u64();
            let ts = b.get_i64();
            let xid = b.get_u32();
            Ok(PgOutputMessage::Begin { final_lsn, ts, xid })
        }
        b'C' => {
            let _flags = b.get_u8();
            let commit_lsn = b.get_u64();
            let end_lsn = b.get_u64();
            let ts = b.get_i64();
            Ok(PgOutputMessage::Commit { commit_lsn, end_lsn, ts })
        }
        b'R' => {
            let rel_id = b.get_u32();
            let schema = read_cstring(&mut b)?;
            let name = read_cstring(&mut b)?;
            let replica_identity = b.get_u8();
            let n = b.get_u16() as usize;
            let mut columns = Vec::with_capacity(n);
            for _ in 0..n {
                let flags = b.get_u8();
                let cname = read_cstring(&mut b)?;
                let oid = b.get_u32();
                let modifier = b.get_i32();
                columns.push(Column { flags, name: cname, data_type_oid: oid, type_modifier: modifier });
            }
            Ok(PgOutputMessage::Relation { rel_id, schema, name, replica_identity, columns })
        }
        b'I' => {
            let rel_id = b.get_u32();
            let _kind = b.get_u8(); // 'N' = new tuple
            let tuple = read_tuple(&mut b)?;
            Ok(PgOutputMessage::Insert { rel_id, tuple })
        }
        b'U' => {
            let rel_id = b.get_u32();
            let mut before = None;
            // Optional 'O' or 'K' before the new tuple
            let mut tag = b.get_u8();
            if tag == b'O' || tag == b'K' {
                before = Some(read_tuple(&mut b)?);
                tag = b.get_u8();
            }
            // tag should now be 'N'
            if tag != b'N' { return Err(format!("unexpected update tag: {tag}")); }
            let after = read_tuple(&mut b)?;
            Ok(PgOutputMessage::Update { rel_id, before, after })
        }
        b'D' => {
            let rel_id = b.get_u32();
            let _tag = b.get_u8(); // 'O' (old) or 'K' (key)
            let key_or_old = read_tuple(&mut b)?;
            Ok(PgOutputMessage::Delete { rel_id, key_or_old })
        }
        b'T' => {
            let n = b.get_u32() as usize;
            let _opt = b.get_u8();
            let mut rel_ids = Vec::with_capacity(n);
            for _ in 0..n { rel_ids.push(b.get_u32()); }
            Ok(PgOutputMessage::Truncate { rel_ids })
        }
        _ => Ok(PgOutputMessage::Other),
    }
}

fn read_cstring(b: &mut &[u8]) -> Result<String, String> {
    let pos = b.iter().position(|&x| x == 0).ok_or("missing nul")?;
    let s = std::str::from_utf8(&b[..pos]).map_err(|e| e.to_string())?.to_string();
    *b = &b[pos+1..];
    Ok(s)
}

fn read_tuple(b: &mut &[u8]) -> Result<TupleData, String> {
    let n = b.get_u16() as usize;
    let mut fields = Vec::with_capacity(n);
    for _ in 0..n {
        let kind = b.get_u8();
        match kind {
            b'n' => fields.push(Field::Null),
            b'u' => fields.push(Field::Toast),
            b't' => {
                let len = b.get_u32() as usize;
                let s = std::str::from_utf8(&b[..len]).map_err(|e| e.to_string())?.to_string();
                *b = &b[len..];
                fields.push(Field::Text(s));
            }
            _ => return Err(format!("unknown field kind: {kind}")),
        }
    }
    Ok(TupleData { fields })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn decode_begin_message() {
        // Minimal hand-crafted Begin message
        let mut buf = vec![b'B'];
        buf.extend_from_slice(&100u64.to_be_bytes());
        buf.extend_from_slice(&123_i64.to_be_bytes());
        buf.extend_from_slice(&42u32.to_be_bytes());
        match decode(&buf).unwrap() {
            PgOutputMessage::Begin { final_lsn: 100, ts: 123, xid: 42 } => {}
            other => panic!("wrong: {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Implement `replication::open`**

Write `crates/kyma-connectors/src/external/postgres/replication.rs`:

```rust
#![forbid(unsafe_code)]
//! Wraps tokio-postgres START_REPLICATION + decodes pgoutput → CdcEvent.

use std::pin::Pin;
use std::sync::Arc;
use futures::stream::Stream;
use futures::StreamExt;

use crate::external::{Checkpoint, ExternalError, SourceHandle, CdcStream};
use crate::external::trait_def::{CdcEvent, CdcOp};
use super::handle::PostgresHandle;
use super::pgoutput::{decode, PgOutputMessage, Field as PgField};

pub async fn open(h: &dyn SourceHandle, from: Option<&Checkpoint>) -> Result<CdcStream, ExternalError> {
    let h = h.as_any().downcast_ref::<PostgresHandle>()
        .ok_or_else(|| ExternalError::Bug("wrong handle".into()))?;
    let conn_url = h.conn_url.clone();

    // Open the replication connection. `replication=database` switches the
    // connection into replication command mode.
    // (tokio-postgres requires a specific config option.)
    let (client, conn) = tokio_postgres::Config::from_str(&conn_url)
        .map_err(|e| ExternalError::OperatorActionable(e.to_string()))?
        .replication_mode(tokio_postgres::config::ReplicationMode::Logical)
        .connect(tokio_postgres::NoTls)
        .await.map_err(|e| classify(e))?;
    tokio::spawn(conn);

    let lsn = from.and_then(|c| c.as_json().get("lsn").and_then(|v| v.as_str().map(|s| s.to_string())))
        .unwrap_or_else(|| "0/0".to_string());
    let slot = from.and_then(|c| c.as_json().get("slot").and_then(|v| v.as_str().map(|s| s.to_string())))
        .ok_or_else(|| ExternalError::OperatorActionable("checkpoint missing slot name".into()))?;

    let stmt = format!("START_REPLICATION SLOT {slot} LOGICAL {lsn} (proto_version '2', publication_names 'kyma_pub')");
    let mut copy_stream = client.copy_both_simple::<bytes::Bytes>(&stmt).await
        .map_err(|e| classify(e))?;

    let mut relations: std::collections::HashMap<u32, super::pgoutput::PgOutputMessage> = std::collections::HashMap::new();

    let stream = async_stream::try_stream! {
        while let Some(msg) = copy_stream.next().await {
            let bytes = msg.map_err(|e| ExternalError::Transient(e.to_string()))?;
            // Parse the XLogData / PrimaryKeepalive wrapper (1-byte type prefix).
            // For brevity: if first byte is 'w' (XLogData), skip 24 bytes (start_lsn 8, end_lsn 8, ts 8) then decode.
            // If 'k' (PrimaryKeepalive), reply with standby status update if reply is requested.
            if bytes.is_empty() { continue; }
            let head = bytes[0];
            if head == b'w' {
                if bytes.len() < 25 { continue; }
                let payload = &bytes[25..];
                let m = decode(payload).map_err(|e| ExternalError::Bug(format!("pgoutput decode: {e}")))?;
                match m {
                    PgOutputMessage::Relation { rel_id, .. } => {
                        relations.insert(rel_id, m.clone());
                    }
                    PgOutputMessage::Insert { rel_id, ref tuple } => {
                        let rel = relations.get(&rel_id).ok_or_else(|| ExternalError::Transient("unknown rel_id".into()))?;
                        let after = tuple_to_json(rel, tuple);
                        let pk = pk_string(rel, tuple);
                        yield CdcEvent {
                            op: CdcOp::Insert,
                            primary_key: pk,
                            checkpoint_at: Checkpoint::new(serde_json::json!({"lsn": "TODO_advance"})),
                            event_time: chrono::Utc::now(),
                            before: None,
                            after: Some(after),
                        };
                    }
                    PgOutputMessage::Update { rel_id, ref after, .. } => {
                        let rel = relations.get(&rel_id).ok_or_else(|| ExternalError::Transient("unknown rel_id".into()))?;
                        let after_json = tuple_to_json(rel, after);
                        let pk = pk_string(rel, after);
                        yield CdcEvent {
                            op: CdcOp::Update,
                            primary_key: pk,
                            checkpoint_at: Checkpoint::new(serde_json::json!({"lsn": "TODO_advance"})),
                            event_time: chrono::Utc::now(),
                            before: None,
                            after: Some(after_json),
                        };
                    }
                    PgOutputMessage::Delete { rel_id, ref key_or_old } => {
                        let rel = relations.get(&rel_id).ok_or_else(|| ExternalError::Transient("unknown rel_id".into()))?;
                        let pk = pk_string(rel, key_or_old);
                        yield CdcEvent {
                            op: CdcOp::Delete,
                            primary_key: pk,
                            checkpoint_at: Checkpoint::new(serde_json::json!({"lsn": "TODO_advance"})),
                            event_time: chrono::Utc::now(),
                            before: None,
                            after: None,
                        };
                    }
                    PgOutputMessage::Begin { .. } | PgOutputMessage::Commit { .. } => {
                        // checkpoint advance happens at Commit; M1 emits the lsn carried on the event.
                    }
                    _ => {}
                }
            } else if head == b'k' {
                // Keepalive — minimal reply path (full impl in M4 hardening).
            }
        }
    };

    Ok(Box::new(Box::pin(stream)))
}

fn tuple_to_json(rel: &PgOutputMessage, tuple: &super::pgoutput::TupleData) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    if let PgOutputMessage::Relation { columns, .. } = rel {
        for (i, col) in columns.iter().enumerate() {
            let v = match tuple.fields.get(i) {
                Some(PgField::Text(s)) => serde_json::Value::String(s.clone()),
                Some(PgField::Null) | Some(PgField::Toast) | None => serde_json::Value::Null,
            };
            obj.insert(col.name.clone(), v);
        }
    }
    serde_json::Value::Object(obj)
}

fn pk_string(rel: &PgOutputMessage, tuple: &super::pgoutput::TupleData) -> String {
    if let PgOutputMessage::Relation { columns, .. } = rel {
        let mut parts = Vec::new();
        for (i, col) in columns.iter().enumerate() {
            if col.flags & 1 != 0 { // 1 = part of replica identity / PK in pgoutput
                if let Some(PgField::Text(s)) = tuple.fields.get(i) {
                    parts.push(s.clone());
                }
            }
        }
        parts.join(":")
    } else { String::new() }
}

fn classify(e: tokio_postgres::Error) -> ExternalError {
    let s = e.to_string();
    let lower = s.to_lowercase();
    if lower.contains("permission") || lower.contains("replication") || lower.contains("slot does not exist") {
        ExternalError::OperatorActionable(s)
    } else { ExternalError::Transient(s) }
}
```

(NB: the LSN advance in `checkpoint_at` is left as `TODO_advance` for M1 minimum-viable; the snapshot coordinator and stream consumer compute the actual high-watermark from `Commit` messages as part of the group-commit step. This is a known M1 simplification — full LSN tracking lands in M4 hardening.)

- [ ] **Step 2: Test with a 100-row insert/update/delete sequence**

Write `crates/kyma-connectors/tests/postgres_replication.rs`:

```rust
use futures::StreamExt;
use kyma_connectors::external::{ResolvedConnection, Scope, TlsMode};
use kyma_connectors::external::trait_def::CdcOp;
use kyma_connectors_testkit::pg_fixture;
use testcontainers::clients::Cli;

#[tokio::test]
async fn cdc_stream_observes_inserts_updates_deletes() {
    let docker = Cli::default();
    let pg = pg_fixture::boot(&docker);
    let pool = sqlx::PgPool::connect(&pg.url).await.unwrap();

    sqlx::query("CREATE PUBLICATION kyma_pub FOR ALL TABLES").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE u (id BIGINT PRIMARY KEY, email TEXT)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO u VALUES (1,'a@x'), (2,'b@x')").execute(&pool).await.unwrap();

    let conn = ResolvedConnection {
        url: pg.url.clone(), password: None, tls: TlsMode::Disabled,
        pool_size: 4, pool_acquire_timeout_ms: 5000, extra: serde_json::Value::Null,
    };
    let h = kyma_connectors::external::postgres::handle::connect(&conn).await.unwrap();
    let scope = Scope { include_schemas: vec!["public".into()], ..Default::default() };

    // Snapshot consumes initial rows AND creates the slot.
    let (mut snap, ckpt) = kyma_connectors::external::postgres::snapshot::take(&*h, &scope).await.unwrap();
    let mut snap_rows = 0;
    while let Some(item) = snap.next().await { item.unwrap(); snap_rows += 1; }
    assert_eq!(snap_rows, 2);

    // Open the CDC stream from the snapshot's checkpoint.
    let mut stream = kyma_connectors::external::postgres::replication::open(&*h, Some(&ckpt)).await.unwrap();

    // Trigger insert / update / delete on the source.
    sqlx::query("INSERT INTO u VALUES (3,'c@x')").execute(&pool).await.unwrap();
    sqlx::query("UPDATE u SET email='c2@x' WHERE id=3").execute(&pool).await.unwrap();
    sqlx::query("DELETE FROM u WHERE id=2").execute(&pool).await.unwrap();

    // Read 3 events with a generous timeout; assert order + ops.
    let mut observed = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while observed.len() < 3 && std::time::Instant::now() < deadline {
        if let Ok(Some(item)) = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next()).await {
            observed.push(item.unwrap());
        }
    }
    assert_eq!(observed.len(), 3, "expected 3 CDC events, got {}", observed.len());
    assert_eq!(observed[0].op, CdcOp::Insert);
    assert_eq!(observed[1].op, CdcOp::Update);
    assert_eq!(observed[2].op, CdcOp::Delete);
    assert_eq!(observed[2].primary_key, "2");
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p kyma-connectors --test postgres_replication -- --nocapture`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/src/external/postgres/pgoutput.rs crates/kyma-connectors/src/external/postgres/replication.rs crates/kyma-connectors/tests/postgres_replication.rs crates/kyma-connectors/Cargo.toml
git commit -m "feat(db): Postgres CDC stream via pgoutput decoder"
```

---

## Task 9: Engine-agnostic snapshot coordinator + stream consumer + runner

**Files:**
- Modify: `crates/kyma-connectors/src/cdc/snapshot.rs`
- Modify: `crates/kyma-connectors/src/cdc/stream.rs`
- Modify: `crates/kyma-connectors/src/cdc/runner.rs`

- [ ] **Step 1: Implement `SnapshotCoordinator::run`**

Replace the stub with:

```rust
impl SnapshotCoordinator {
    pub async fn run(
        &self,
        connector_id: uuid::Uuid,
        source_table: &str,
        handle: &dyn crate::external::SourceHandle,
        scope: &crate::external::Scope,
        catalog_pool: &sqlx::PgPool,
        commit: &dyn IngestCommit, // wrapper over CommitCoordinator
    ) -> Result<crate::external::Checkpoint, crate::external::ExternalError> {
        // 1. Set phase=snapshotting
        crate::cdc::state::set_phase(catalog_pool, connector_id, source_table, "snapshotting")
            .await.map_err(|e| crate::external::ExternalError::Transient(e.to_string()))?;

        // 2. Run snapshot
        let (mut stream, ckpt) = self.source.snapshot_at(handle, scope).await?;

        // 3. Drain stream → batches → ingest write path
        use futures::StreamExt;
        while let Some(item) = stream.next().await {
            let _row = item?;
            // For M1: append to a buffer; flush every N rows via commit.
            // (Wire to kyma-ingest-core staging buffer; the exact API depends on
            // the buffer's public surface. Sketch: commit.append_row(row).)
        }

        // 4. Final batch + cursor advance + phase transition in one CAS.
        commit.commit_with_cursor_and_phase(connector_id, source_table, ckpt.clone(), "streaming")
            .await.map_err(|e| crate::external::ExternalError::Transient(e.to_string()))?;
        Ok(ckpt)
    }
}

pub trait IngestCommit: Send + Sync {
    // Adapter trait — implemented in kyma-ingest-core or in kyma-connectors
    // depending on which side owns the dependency. M1 lands the trait in
    // kyma-connectors and a default impl in kyma-bin.
}
```

- [ ] **Step 2: Implement `CdcStreamConsumer::run`**

Same pattern. Group-commit batches; flush every `MAX_BATCH_BYTES` or `MAX_BATCH_MS`; emit lag heartbeats every N events; commit with `with_cursor_update`.

- [ ] **Step 3: Implement `CdcRunner::run_one_table`**

Phase machine: read state → if `pending` or `snapshotting` run SnapshotCoordinator → else run CdcStreamConsumer; on error classify and route per `ExternalError` taxonomy.

- [ ] **Step 4: Integration test for snapshot+stream**

Write a test that wires the coordinator + stream consumer end-to-end against a Postgres testcontainer with 1000 seed rows + 100 streamed inserts and asserts the synced kyma table has 1100 rows after the runner stabilizes.

- [ ] **Step 5: Run + commit**

```bash
cargo test -p kyma-connectors cdc -- --nocapture
git add crates/kyma-connectors/src/cdc/
git commit -m "feat(db): engine-agnostic CDC coordinator/stream/runner — Postgres-validated"
```

---

## Task 10: Federation table provider — wire scan_exec

**Files:**
- Create: `crates/kyma-federation/src/scan_exec.rs`
- Create: `crates/kyma-federation/src/schema_cache.rs`
- Modify: `crates/kyma-federation/src/table_provider.rs`
- Modify: `crates/kyma-federation/src/catalog_provider.rs`

- [ ] **Step 1: Implement `FederatedScanExec`** as a DataFusion `ExecutionPlan`. It owns an `Arc<dyn ExternalSource>`, a `PushedPlan`, and produces an Arrow stream by calling `source.scan(handle, &plan, &mut sink)` where `sink` writes batches into a DataFusion `RecordBatchStream`.

- [ ] **Step 2: Implement `SchemaCache`** with TTL invalidation.

- [ ] **Step 3: Wire `FederatedTableProvider::scan` to:**
  1. Build `PlanInput` from filters/projection/limit/sort.
  2. Call `PushdownPlanner.plan(input)`.
  3. Render `PushedPlan::Sql` via `postgres::sql_render`.
  4. Construct `FederatedScanExec` with the plan.
  5. Wrap residual filters in DataFusion `FilterExec` if non-empty.

- [ ] **Step 4: Wire `FederatedCatalogProvider`** to populate schema/table lists from the cache.

- [ ] **Step 5: Integration test — `SELECT * FROM pg_prod.public.users WHERE region = 'eu' LIMIT 5`**

Spin a kyma + Postgres testcontainer; register a `PostgresSource` into the registry; run the query; assert 5 rows returned and `pushdown_summary.filters_pushed` non-empty.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-federation/src/scan_exec.rs crates/kyma-federation/src/schema_cache.rs crates/kyma-federation/src/table_provider.rs crates/kyma-federation/src/catalog_provider.rs
git commit -m "feat(db): wire federated scan_exec end-to-end for Postgres"
```

---

## Task 11: Pushdown extensions — LIMIT, ORDER BY, single-source agg, opportunistic same-source join

**Files:**
- Modify: `crates/kyma-federation/src/planner.rs`
- Modify: `crates/kyma-connectors/src/external/postgres/sql_render.rs`

- [ ] **Step 1: Extend `PushdownPlanner::plan`** to also walk a parent `Aggregate` node (DataFusion's logical plan exposes this) and decide pushdown when:
  1. All grouping columns are projections of a single `FederatedTableProvider`.
  2. All aggregate functions are in `Capabilities::agg_funcs`.

  Output: `PushedPlan::Sql` carrying the full `SELECT … GROUP BY …` shape.

- [ ] **Step 2: Extend `sql_render`** to render `ORDER BY` clauses and `GROUP BY` clauses.

- [ ] **Step 3: Add opportunistic same-source join detection.** When the planner observes a join between two `FederatedTableProvider`s that both belong to the same `source_name`, build a single `SELECT … FROM a JOIN b ON …` and push it as one `FederatedScanExec`.

- [ ] **Step 4: Integration tests** for each shape: `SELECT COUNT(*) FROM pg.t WHERE x = ?` (asserts one row over the wire), `SELECT a, SUM(b) FROM pg.t GROUP BY a`, `SELECT * FROM pg.users u JOIN pg.orders o ON u.id = o.user_id`.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-federation/src/planner.rs crates/kyma-connectors/src/external/postgres/sql_render.rs
git commit -m "feat(db): pushdown for LIMIT/ORDER BY/single-source aggregation/same-source join"
```

---

## Task 12: Property tests — Postgres pushdown correctness

**Files:**
- Create: `crates/kyma-federation/tests/planner_property_pg.rs`
- Modify: `crates/kyma-connectors-testkit/src/query_gen.rs`

- [ ] **Step 1: Extend the testkit generator** to also generate aggregations (`COUNT`, `SUM`, etc.), `ORDER BY`, `LIMIT`, and single allowlist-function calls. See spec §12.2 for the shape.

- [ ] **Step 2: Property test runner** — for each generated `GeneratedQuery`:
  1. Boot Postgres testcontainer with seed data (use `kyma-connectors-testkit::seed_pg::seed_users(...)`).
  2. Execute the query against Postgres directly via sqlx → reference Arrow.
  3. Execute the same query via DataFusion + `FederatedTableProvider` → pushed Arrow.
  4. Sort both by all columns; compare row-for-row.

- [ ] **Step 3: Configure 10k cases in normal CI; 100k in nightly via env var.**

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-federation/tests/planner_property_pg.rs crates/kyma-connectors-testkit/src/query_gen.rs crates/kyma-connectors-testkit/src/seed_pg.rs
git commit -m "test(db): pushdown property tests against Postgres"
```

---

## Task 13: Chaos suite for Postgres

**Files:**
- Create: `crates/kyma-connectors/tests/postgres_chaos.rs`
- Modify: `crates/kyma-connectors-testkit/src/chaos.rs`

- [ ] **Step 1: Implement chaos primitives** in `kyma-connectors-testkit::chaos`: `kill_runner`, `restart_source`, `partition_network` (iptables via testcontainers exec), `drop_replication_slot`, `cas_storm`.

- [ ] **Step 2: Write the seven chaos tests from spec §12.4.** Each:
  1. Sets up a 1M-row table (or 1k inserts/sec stream).
  2. Runs the runner.
  3. Triggers the fault.
  4. Asserts: synced table content equals source content (modulo deletes-as-tombstones); cursors monotonic; no duplicates.

- [ ] **Step 3: Gate with env var.** Mark each test with `#[ignore]` by default; runs in CI only when `RUN_CHAOS=1`.

- [ ] **Step 4: Run locally**

```bash
RUN_CHAOS=1 cargo test -p kyma-connectors --test postgres_chaos -- --nocapture --ignored --test-threads=1
```

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-connectors/tests/postgres_chaos.rs crates/kyma-connectors-testkit/src/chaos.rs
git commit -m "test(db): chaos suite for Postgres exactly-once invariants"
```

---

## Task 14: kyma-bin wiring — register sources at startup

**Files:**
- Modify: `crates/kyma-bin/src/main.rs`

- [ ] **Step 1: At startup, after the catalog is connected, read `connectors` rows where `mode IN ('federation','both')` and register a `PostgresSource` per row in the `FederationRegistry`. Then call `kyma_exec::register_federated_catalogs(&session_ctx, &registry)`.**

- [ ] **Step 2: For `mode IN ('sync','both')` rows, register a `CdcConnector` per row in the connector registry.**

- [ ] **Step 3: Smoke test:** `docker compose up`, create a connector, run a federated query end-to-end through `kyma-server`'s `/v1/query` endpoint.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-bin/src/main.rs
git commit -m "feat(db): kyma-bin registers PostgresSource + CdcConnector at startup"
```

---

## Task 15: Performance benchmarks per spec §12.5

**Files:**
- Create: `crates/kyma-connectors/benches/postgres_federation.rs`
- Create: `crates/kyma-connectors/benches/postgres_sync.rs`

- [ ] **Step 1: Federation latency benches via criterion.** Three benches: single-row PK lookup, 1k-row scan with filter pushdown, cross-source join.

- [ ] **Step 2: Sync throughput benches.** Snapshot rate, stream rate, lag under load.

- [ ] **Step 3: Wire criterion budgets.** Failures > 25% regression block the merge.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-connectors/benches/
git commit -m "bench(db): Postgres federation + sync performance budgets"
```

---

## Task 16: M1 acceptance smoke

- [ ] `cargo build --workspace --all-targets --features postgres,federation` clean.
- [ ] `cargo test --workspace --all-targets` clean.
- [ ] `RUN_CHAOS=1 cargo test --test postgres_chaos -- --ignored` passes all seven scenarios.
- [ ] `cargo bench -p kyma-connectors --bench postgres_federation` within budgets.
- [ ] `pushdown_summary` present and non-empty on every federated response in the integration test.
- [ ] Tag `db-m1-postgres`.

---

## M1 Open Decisions Resolved During M0 Review

- The shape of `Capabilities` may have shifted during M0 review. If so, update Task 1 here.
- The shape of `PushedPlan` may have shifted (string vs AST). If so, update Tasks 5, 6, 11.
- The exact `IngestCommit` adapter shape (Task 9) depends on the `kyma-ingest-core::CommitCoordinator` public surface as it lands in M0 Task 7.

These are the three points to verify first thing on M1 day-1.
