# Cloud Slice 0 — Engine-side Tenancy Retrofit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retrofit the kyma engine for multi-tenancy: every catalog row, every storage path, and every authenticated request carries a `tenant_id`, with two pluggable auth backends (env-token for self-hosted, db-backed behind cargo feature for cloud).

**Architecture:** Additive Postgres migration adds `tenant_id UUID NOT NULL` to nine catalog tables and rebuilds compound unique constraints as `(tenant_id, name)`. The `Catalog` trait grows tenant-aware variants of every `(database, name)` lookup; existing methods become thin wrappers that pass `DEFAULT_TENANT`. Storage paths get a `<tenant_id>/` segment via `TelemetryFormat::with_tenant`. Auth refactors into an `AuthBackend` trait — `EnvAuthBackend` (default) preserves `KYMA_AUTH_TOKENS` semantics and yields `DEFAULT_TENANT`; `DbAuthBackend` (cargo feature `cloud-auth`) reads `api_tokens` from Postgres and yields the workspace's tenant. A request extension `TenantId` flows from middleware through every handler. Connector secrets schema lands now (encrypted_with_kms_key_id columns) but actual KMS wiring is deferred to Slice 2.

**Tech Stack:** Rust 1.x workspace, sqlx 0.7 + Postgres 16, Axum 0.7, tower-http, async-trait, uuid 1.x, testcontainers (Docker-required integration tests).

---

## Scope

**Includes (Slice 0):**
- Migration `007_tenant_id.sql` adding `tenant_id` columns + indexes + connector KMS schema columns.
- `kyma-core` adds `TenantId` newtype + `DEFAULT_TENANT` constant.
- `Catalog` trait grows `*_in_tenant` methods; legacy methods delegate.
- `kyma-storage` / `kyma-format-tlm` extent paths get tenant segment.
- `kyma-server` refactors auth into `AuthBackend` trait + `EnvAuthBackend` + (feature-gated) `DbAuthBackend`.
- Middleware injects `TenantId` request extension; handlers consume it.
- Connector direct-SQL helpers retrofitted to bind tenant_id (gap caught during plan self-review).
- Verification gate: cross-tenant isolation integration test.

**Explicitly excludes (per spec + plan-agent review):**
- Per-tenant query budgets / cancellation — Slice 2.5.
- Cloud control plane (workspaces, magic link, Stripe) — Slice 2.
- MCP server — Slice 1a (parallel plan).
- Real KMS integration — schema columns only; wiring lands in Slice 2.

---

## File Structure

**Create:**
- `crates/kyma-catalog/migrations/007_tenant_id.sql` — additive multi-table migration.
- `crates/kyma-core/src/tenant.rs` — `TenantId`, `DEFAULT_TENANT`, request-extension type.
- `crates/kyma-server/src/auth/mod.rs` — module shim re-exporting `Role`, `AuthBackend`, `AuthLayer`.
- `crates/kyma-server/src/auth/backend.rs` — `AuthBackend` trait + `Principal`.
- `crates/kyma-server/src/auth/env_backend.rs` — moved/refactored `EnvAuthBackend` (preserves `KYMA_AUTH_TOKENS`).
- `crates/kyma-server/src/auth/db_backend.rs` — `DbAuthBackend` (cargo feature `cloud-auth`).
- `crates/kyma-server/src/auth/middleware.rs` — `require_role_middleware` reading from any `AuthBackend`.
- `crates/kyma-catalog/tests/tenant_isolation_it.rs` — same-name-different-tenant gate test.
- `crates/kyma-server/tests/auth_backends_it.rs` — `EnvAuthBackend` + `DbAuthBackend` test.

**Modify:**
- `crates/kyma-core/src/lib.rs` — export `tenant` module.
- `crates/kyma-core/src/catalog.rs` — add `*_in_tenant` methods; default impls delegate to legacy.
- `crates/kyma-catalog/src/lib.rs` — every read/write now takes a `tenant_id`; legacy methods call new ones with `DEFAULT_TENANT`.
- `crates/kyma-catalog/src/snapshot.rs` — extent inserts persist `tenant_id`.
- `crates/kyma-format-tlm/src/lib.rs` — add `TelemetryFormat::with_tenant`.
- `crates/kyma-format-tlm/src/writer.rs` — `format_extent_path` includes tenant segment.
- `crates/kyma-server/src/lib.rs` — replace single `auth` module with `auth/` directory; mount middleware via `AuthLayer::layer`.
- `crates/kyma-server/Cargo.toml` — add `cloud-auth` feature + sqlx Postgres feature gating.
- `crates/kyma-bin/src/main.rs` — pick `AuthBackend` impl from env; pass tenant into ingest/query.
- `crates/kyma-connectors/src/catalog_sql.rs` — retrofit direct-SQL helpers for tenant_id.
- `crates/kyma-connectors/src/admin.rs` — thread tenant from `Principal` request extension.

**Test:**
- `crates/kyma-catalog/tests/tenant_isolation_it.rs` — new (cross-tenant gate).
- `crates/kyma-catalog/tests/catalog_it.rs` — update fixtures to pass tenant.
- `crates/kyma-server/tests/auth_backends_it.rs` — new.
- `crates/kyma-server/tests/cleanup_http.rs` — update fixture inserts to pass tenant.

---

## Tasks

### Task 1: Add `TenantId` type to kyma-core

**Files:**
- Create: `crates/kyma-core/src/tenant.rs`
- Modify: `crates/kyma-core/src/lib.rs`

- [ ] **Step 1: Write the test**

Create `crates/kyma-core/src/tenant.rs`:

```rust
//! Tenant identity type — load-bearing for cloud's multi-workspace isolation.
//!
//! Every catalog row, every storage object, and every authenticated request
//! carries a [`TenantId`]. Self-hosted deployments use [`DEFAULT_TENANT`];
//! cloud deployments mint a fresh UUID per workspace.

use uuid::Uuid;

/// Identifier of a tenant (≅ a cloud workspace, or the single self-hosted owner).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct TenantId(pub Uuid);

impl TenantId {
    pub const fn from_uuid(u: Uuid) -> Self { Self(u) }
    pub const fn as_uuid(&self) -> &Uuid { &self.0 }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// All-zero UUID used by self-hosted deployments and as the migration default.
pub const DEFAULT_TENANT: TenantId =
    TenantId::from_uuid(Uuid::from_bytes([0u8; 16]));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tenant_is_all_zeros() {
        assert_eq!(DEFAULT_TENANT.as_uuid().as_bytes(), &[0u8; 16]);
    }

    #[test]
    fn tenant_id_displays_as_hyphenated_uuid() {
        let t = DEFAULT_TENANT;
        assert_eq!(t.to_string(), "00000000-0000-0000-0000-000000000000");
    }
}
```

- [ ] **Step 2: Wire module into lib.rs**

In `crates/kyma-core/src/lib.rs`, locate the existing `pub mod` declarations (top of file) and add:

```rust
pub mod tenant;
pub use tenant::{TenantId, DEFAULT_TENANT};
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p kyma-core tenant`
Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-core/src/tenant.rs crates/kyma-core/src/lib.rs
git commit -m "feat(core): add TenantId newtype and DEFAULT_TENANT constant"
```

---

### Task 2: Migration `007_tenant_id.sql`

**Files:**
- Create: `crates/kyma-catalog/migrations/007_tenant_id.sql`

- [ ] **Step 1: Write migration**

Create `crates/kyma-catalog/migrations/007_tenant_id.sql`:

```sql
-- 007_tenant_id.sql
-- Add tenant_id to every tenant-scoped table for the cloud retrofit.
--
-- Strategy:
--   1. ADD COLUMN with DEFAULT '00000000-...' so existing rows backfill in place.
--   2. Drop the DEFAULT after backfill so future inserts must supply tenant_id.
--   3. Replace single-column UNIQUE constraints with (tenant_id, name) where the
--      cloud product needs same-name databases / connectors / dashboards across
--      workspaces.
--   4. Index every tenant_id column so per-tenant queries hit indices.
--   5. Add connector KMS schema columns now (Slice 2 wires the actual encryption).

-- 1. databases
ALTER TABLE databases ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE databases ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE databases DROP CONSTRAINT IF EXISTS databases_name_key;
ALTER TABLE databases ADD CONSTRAINT databases_tenant_name_uniq UNIQUE (tenant_id, name);
CREATE INDEX databases_tenant_idx ON databases (tenant_id);

-- 2. tables
ALTER TABLE tables ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE tables ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX tables_tenant_idx ON tables (tenant_id);

-- 3. snapshots
ALTER TABLE snapshots ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE snapshots ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX snapshots_tenant_idx ON snapshots (tenant_id);

-- 4. schema_snapshots
ALTER TABLE schema_snapshots ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE schema_snapshots ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX schema_snapshots_tenant_idx ON schema_snapshots (tenant_id);

-- 5. manifests
ALTER TABLE manifests ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE manifests ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX manifests_tenant_idx ON manifests (tenant_id);

-- 6. extents
ALTER TABLE extents ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE extents ALTER COLUMN tenant_id DROP DEFAULT;
DROP INDEX IF EXISTS extents_live;
CREATE INDEX extents_live ON extents (tenant_id, table_id) WHERE deleted_at IS NULL;
CREATE INDEX extents_tenant_idx ON extents (tenant_id);

-- 7. ingest_ledger
ALTER TABLE ingest_ledger ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE ingest_ledger ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE ingest_ledger DROP CONSTRAINT IF EXISTS ingest_ledger_pkey;
ALTER TABLE ingest_ledger ADD CONSTRAINT ingest_ledger_pkey
    PRIMARY KEY (tenant_id, idempotency_key);

-- 8. background_tasks
ALTER TABLE background_tasks ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE background_tasks ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX background_tasks_tenant_idx ON background_tasks (tenant_id);

-- 9. dashboards + dashboard_panels
ALTER TABLE dashboards ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE dashboards ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX dashboards_tenant_idx ON dashboards (tenant_id);
ALTER TABLE dashboard_panels ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE dashboard_panels ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX dashboard_panels_tenant_idx ON dashboard_panels (tenant_id);

-- 10. connectors + connector_cursors + connector_leases
ALTER TABLE connectors ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE connectors ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE connectors ADD COLUMN encrypted_with_kms_key_id text;
ALTER TABLE connectors ADD COLUMN encrypted_secrets_blob bytea;
ALTER TABLE connectors DROP CONSTRAINT IF EXISTS connectors_name_key;
ALTER TABLE connectors ADD CONSTRAINT connectors_tenant_name_uniq UNIQUE (tenant_id, name);
CREATE INDEX connectors_tenant_idx ON connectors (tenant_id);
ALTER TABLE connector_cursors ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE connector_cursors ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX connector_cursors_tenant_idx ON connector_cursors (tenant_id);
ALTER TABLE connector_leases ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE connector_leases ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX connector_leases_tenant_idx ON connector_leases (tenant_id);

-- 11. agent surface
ALTER TABLE agent_runs ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_runs ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX agent_runs_tenant_idx ON agent_runs (tenant_id, started_at DESC);
ALTER TABLE agent_sessions ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_sessions ALTER COLUMN tenant_id DROP DEFAULT;
CREATE INDEX agent_sessions_tenant_idx ON agent_sessions (tenant_id);
ALTER TABLE agent_session_turns ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_session_turns ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE agent_replay_cache ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE agent_replay_cache ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE agent_replay_cache DROP CONSTRAINT IF EXISTS agent_replay_cache_pkey;
ALTER TABLE agent_replay_cache ADD CONSTRAINT agent_replay_cache_pkey
    PRIMARY KEY (tenant_id, cache_key);

-- 12. column_metadata, schema_embeddings
ALTER TABLE column_metadata ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE column_metadata ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE column_metadata DROP CONSTRAINT IF EXISTS column_metadata_pkey;
ALTER TABLE column_metadata ADD CONSTRAINT column_metadata_pkey
    PRIMARY KEY (tenant_id, database, table_name, column_name);

ALTER TABLE schema_embeddings ADD COLUMN tenant_id uuid NOT NULL
    DEFAULT '00000000-0000-0000-0000-000000000000';
ALTER TABLE schema_embeddings ALTER COLUMN tenant_id DROP DEFAULT;
DROP INDEX IF EXISTS schema_embeddings_uniq_table;
DROP INDEX IF EXISTS schema_embeddings_uniq_column;
DROP INDEX IF EXISTS schema_embeddings_db;
CREATE UNIQUE INDEX schema_embeddings_uniq_table
    ON schema_embeddings (tenant_id, database, table_name, model_id)
    WHERE column_name IS NULL;
CREATE UNIQUE INDEX schema_embeddings_uniq_column
    ON schema_embeddings (tenant_id, database, table_name, column_name, model_id)
    WHERE column_name IS NOT NULL;
CREATE INDEX schema_embeddings_db
    ON schema_embeddings (tenant_id, database);

-- nodes is intentionally NOT tenant-scoped — node identity is cluster-global.
```

- [ ] **Step 2: Verify migration runs**

Run: `cargo test -p kyma-catalog --test catalog_it create_database_and_table`
Expected: PASS (sqlx::migrate! runs 001-007 against fresh testcontainers Postgres).

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-catalog/migrations/007_tenant_id.sql
git commit -m "feat(catalog): migration 007 adds tenant_id to all tenant-scoped tables"
```

---

### Task 3: Extend `Catalog` trait with tenant-aware methods

**Files:**
- Modify: `crates/kyma-core/src/catalog.rs`

The trait grows new `*_in_tenant` methods. Existing methods get default impls that delegate using `DEFAULT_TENANT`, so callers outside the cloud retrofit (cli, tests) compile unchanged.

- [ ] **Step 1: Add tenant-aware methods to the trait**

In `crates/kyma-core/src/catalog.rs`, after the `Catalog` trait's `as_ref_any` method, insert these method declarations:

```rust
    // ---------------- tenant-aware (cloud) ----------------

    async fn create_database_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        name: &str,
    ) -> Result<DatabaseId>;

    async fn create_table_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database_id: DatabaseId,
        name: &str,
        schema: SchemaRef,
        config: TableConfig,
    ) -> Result<TableId>;

    async fn lookup_table_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
    ) -> Result<TableRef>;

    async fn list_tables_in_database_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
    ) -> Result<Vec<TableRef>>;

    async fn list_databases_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
    ) -> Result<Vec<String>, CatalogError>;

    async fn list_tables_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
    ) -> Result<Vec<String>, CatalogError>;

    async fn get_table_columns_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnInfo>, CatalogError>;

    async fn cleanup_soft_deleted_extents_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        table: &str,
        before: DateTime<Utc>,
    ) -> Result<CleanupResult>;

    async fn lookup_idempotency_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        key: &str,
    ) -> Result<Option<IngestLedgerEntry>>;

    async fn record_idempotency_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        key: &str,
        entry: IngestLedgerEntry,
        ttl: chrono::Duration,
    ) -> Result<Option<IngestLedgerEntry>>;

    async fn create_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        name: &str,
        description: Option<&str>,
    ) -> Result<Dashboard, CatalogError>;

    async fn list_dashboards_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
    ) -> Result<Vec<Dashboard>, CatalogError>;

    async fn get_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        id: uuid::Uuid,
    ) -> Result<Option<DashboardWithPanels>, CatalogError>;

    async fn update_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        id: uuid::Uuid,
        patch: DashboardUpdate,
    ) -> Result<Dashboard, CatalogError>;

    async fn delete_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        id: uuid::Uuid,
    ) -> Result<bool, CatalogError>;

    async fn list_extents_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
        snapshot: SnapshotId,
        prune: &PrunePredicate,
    ) -> Result<Vec<ExtentManifest>>;

    async fn begin_snapshot_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
    ) -> Result<Box<dyn SnapshotTxn>>;
```

- [ ] **Step 2: Convert legacy methods into delegating default impls**

Replace each existing `async fn create_database`, `create_table`, `lookup_table`, `list_tables_in_database`, `list_databases`, `list_tables`, `get_table_columns`, `cleanup_soft_deleted_extents`, `lookup_idempotency`, `record_idempotency`, `create_dashboard`, `list_dashboards`, `get_dashboard`, `update_dashboard`, `delete_dashboard`, `list_extents`, `begin_snapshot` method body with a default body that delegates:

```rust
    async fn create_database(&self, name: &str) -> Result<DatabaseId> {
        self.create_database_in_tenant(crate::tenant::DEFAULT_TENANT, name).await
    }
    async fn lookup_table(&self, database: &str, name: &str) -> Result<TableRef> {
        self.lookup_table_in_tenant(crate::tenant::DEFAULT_TENANT, database, name).await
    }
    async fn list_tables_in_database(&self, database: &str) -> Result<Vec<TableRef>> {
        self.list_tables_in_database_in_tenant(crate::tenant::DEFAULT_TENANT, database).await
    }
    async fn list_databases(&self) -> Result<Vec<String>, CatalogError> {
        self.list_databases_in_tenant(crate::tenant::DEFAULT_TENANT).await
    }
    async fn list_tables(&self, database: &str) -> Result<Vec<String>, CatalogError> {
        self.list_tables_in_tenant(crate::tenant::DEFAULT_TENANT, database).await
    }
    async fn get_table_columns(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnInfo>, CatalogError> {
        self.get_table_columns_in_tenant(crate::tenant::DEFAULT_TENANT, database, table).await
    }
    async fn create_table(
        &self,
        database_id: DatabaseId,
        name: &str,
        schema: SchemaRef,
        config: TableConfig,
    ) -> Result<TableId> {
        self.create_table_in_tenant(crate::tenant::DEFAULT_TENANT, database_id, name, schema, config).await
    }
    async fn cleanup_soft_deleted_extents(
        &self,
        database: &str,
        table: &str,
        before: DateTime<Utc>,
    ) -> Result<CleanupResult> {
        self.cleanup_soft_deleted_extents_in_tenant(
            crate::tenant::DEFAULT_TENANT, database, table, before,
        ).await
    }
    async fn lookup_idempotency(&self, key: &str) -> Result<Option<IngestLedgerEntry>> {
        self.lookup_idempotency_in_tenant(crate::tenant::DEFAULT_TENANT, key).await
    }
    async fn record_idempotency(
        &self,
        key: &str,
        entry: IngestLedgerEntry,
        ttl: chrono::Duration,
    ) -> Result<Option<IngestLedgerEntry>> {
        self.record_idempotency_in_tenant(crate::tenant::DEFAULT_TENANT, key, entry, ttl).await
    }
    async fn create_dashboard(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Dashboard, CatalogError> {
        self.create_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, name, description).await
    }
    async fn list_dashboards(&self) -> Result<Vec<Dashboard>, CatalogError> {
        self.list_dashboards_in_tenant(crate::tenant::DEFAULT_TENANT).await
    }
    async fn get_dashboard(
        &self,
        id: uuid::Uuid,
    ) -> Result<Option<DashboardWithPanels>, CatalogError> {
        self.get_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, id).await
    }
    async fn update_dashboard(
        &self,
        id: uuid::Uuid,
        patch: DashboardUpdate,
    ) -> Result<Dashboard, CatalogError> {
        self.update_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, id, patch).await
    }
    async fn delete_dashboard(&self, id: uuid::Uuid) -> Result<bool, CatalogError> {
        self.delete_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, id).await
    }
    async fn list_extents(
        &self,
        table_id: TableId,
        snapshot: SnapshotId,
        prune: &PrunePredicate,
    ) -> Result<Vec<ExtentManifest>> {
        self.list_extents_in_tenant(crate::tenant::DEFAULT_TENANT, table_id, snapshot, prune).await
    }
    async fn begin_snapshot(&self, table_id: TableId) -> Result<Box<dyn SnapshotTxn>> {
        self.begin_snapshot_in_tenant(crate::tenant::DEFAULT_TENANT, table_id).await
    }
```

Methods NOT delegated this way (because their input ID already pins the tenant): `alter_table_add_column`, `gc_candidates`, `delete_extent_rows`, node management, background-task queue, idempotency cleanup.

- [ ] **Step 3: Type-check**

Run: `cargo check -p kyma-core`
Expected: builds.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-core/src/catalog.rs
git commit -m "feat(core): grow Catalog trait with tenant-aware *_in_tenant methods"
```

---

### Task 4: Implement tenant-aware methods in `PostgresCatalog`

**Files:**
- Modify: `crates/kyma-catalog/src/lib.rs`
- Create: `crates/kyma-catalog/tests/tenant_isolation_it.rs`

`PostgresCatalog` must implement every new `*_in_tenant` method. Each takes a `TenantId`, threads it through every SQL statement (`WHERE tenant_id = $N`, `INSERT INTO ... (tenant_id, ...)`).

- [ ] **Step 1: Write the failing isolation test**

Create `crates/kyma-catalog/tests/tenant_isolation_it.rs`:

```rust
//! Slice 0 verification gate: cross-tenant isolation.

use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::errors::{CatalogError, Error};
use kyma_core::tenant::TenantId;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn fixture() -> (PostgresCatalog, testcontainers::ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_user("kyma")
        .with_password("kyma_dev")
        .with_db_name("kyma")
        .start()
        .await
        .expect("start postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");
    let catalog = PostgresCatalog::connect(&url).await.expect("connect + migrate");
    (catalog, container)
}

#[tokio::test]
async fn same_database_name_under_two_tenants_does_not_cross() {
    let (catalog, _container) = fixture().await;

    let tenant_a = TenantId::from_uuid(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
    let tenant_b = TenantId::from_uuid(Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());

    let db_a = catalog.create_database_in_tenant(tenant_a, "default").await.unwrap();
    let db_b = catalog.create_database_in_tenant(tenant_b, "default").await.unwrap();
    assert_ne!(db_a.as_uuid(), db_b.as_uuid());

    let a_dbs = catalog.list_databases_in_tenant(tenant_a).await.unwrap();
    let b_dbs = catalog.list_databases_in_tenant(tenant_b).await.unwrap();
    assert_eq!(a_dbs, vec!["default".to_string()]);
    assert_eq!(b_dbs, vec!["default".to_string()]);

    let tenant_c = TenantId::from_uuid(Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap());
    let c_dbs = catalog.list_databases_in_tenant(tenant_c).await.unwrap();
    assert!(c_dbs.is_empty());
}

#[tokio::test]
async fn lookup_table_never_crosses_tenants() {
    use arrow_schema::{DataType, Field, Schema};
    use kyma_core::catalog::TableConfig;
    use std::sync::Arc;

    let (catalog, _container) = fixture().await;

    let tenant_a = TenantId::from_uuid(Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap());
    let tenant_b = TenantId::from_uuid(Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap());

    let schema_a = Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]));
    let schema_b = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("payload", DataType::Utf8, true),
    ]));

    let db_a = catalog.create_database_in_tenant(tenant_a, "default").await.unwrap();
    catalog.create_table_in_tenant(tenant_a, db_a, "events", schema_a, TableConfig::default())
        .await.unwrap();

    let db_b = catalog.create_database_in_tenant(tenant_b, "default").await.unwrap();
    catalog.create_table_in_tenant(tenant_b, db_b, "events", schema_b, TableConfig::default())
        .await.unwrap();

    let a = catalog.lookup_table_in_tenant(tenant_a, "default", "events").await.unwrap();
    assert_eq!(a.schema.fields().len(), 1);

    let b = catalog.lookup_table_in_tenant(tenant_b, "default", "events").await.unwrap();
    assert_eq!(b.schema.fields().len(), 2);

    let tenant_c = TenantId::from_uuid(Uuid::parse_str("cccccccc-cccc-cccc-cccc-cccccccccccc").unwrap());
    let err = catalog.lookup_table_in_tenant(tenant_c, "default", "events")
        .await.expect_err("tenant c must not see other tenants' tables");
    match err {
        Error::Catalog(CatalogError::DatabaseNotFound(_)) | Error::Catalog(CatalogError::TableNotFound { .. }) => {}
        other => panic!("expected DatabaseNotFound or TableNotFound, got {other:?}"),
    }
}
```

- [ ] **Step 2: Verify it fails**

Run: `cargo test -p kyma-catalog --test tenant_isolation_it`
Expected: COMPILE FAIL — `create_database_in_tenant` etc. unimplemented.

- [ ] **Step 3: Implement tenant-aware methods**

In `crates/kyma-catalog/src/lib.rs`, replace the existing legacy method bodies inside `impl Catalog for PostgresCatalog` with the new `*_in_tenant` methods (legacy ones are now default impls — remove them from the impl block).

Add `use kyma_core::tenant::TenantId;`.

For each of the 17 methods listed in Task 3, implement the `_in_tenant` variant by copying the existing body and:
- Adding `tenant_id` as first column / first WHERE clause (`WHERE tenant_id = $1 AND ...`)
- Binding `tenant.as_uuid()` first
- Shifting all other bind indices by one
- For INSERT, prepending `tenant_id` to the column list and binding tenant first
- For multi-table JOINs, adding `tenant_id = $1` to every joined table

Example — `create_database_in_tenant`:

```rust
    async fn create_database_in_tenant(
        &self,
        tenant: TenantId,
        name: &str,
    ) -> Result<DatabaseId> {
        let row: (Uuid,) = sqlx::query_as(
            "INSERT INTO databases (tenant_id, name) VALUES ($1, $2) RETURNING id",
        )
        .bind(tenant.as_uuid())
        .bind(name)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?;
        Ok(DatabaseId::from_uuid(row.0))
    }
```

Example — `lookup_table_in_tenant`:

```rust
    async fn lookup_table_in_tenant(
        &self,
        tenant: TenantId,
        database: &str,
        name: &str,
    ) -> Result<TableRef> {
        let row = sqlx::query(
            "SELECT t.id, t.database_id, t.current_snapshot_id, t.schema_snapshot_id,
                    ss.arrow_schema, t.config
             FROM tables t
             JOIN databases d ON d.id = t.database_id
             LEFT JOIN schema_snapshots ss ON ss.id = t.schema_snapshot_id
             WHERE t.tenant_id = $1 AND d.tenant_id = $1
               AND d.name = $2 AND t.name = $3",
        )
        .bind(tenant.as_uuid())
        .bind(database)
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CatalogError::Sql(e.to_string()))?
        .ok_or_else(|| CatalogError::TableNotFound {
            database: database.to_owned(),
            name: name.to_owned(),
        })?;
        // ... rest of body identical to existing lookup_table, extracting columns from row
        let id: Uuid = row.try_get("id").map_err(sql_err)?;
        let database_id: Uuid = row.try_get("database_id").map_err(sql_err)?;
        let current_snapshot_id: Option<Uuid> =
            row.try_get("current_snapshot_id").map_err(sql_err)?;
        let schema_snapshot_id: Option<Uuid> =
            row.try_get("schema_snapshot_id").map_err(sql_err)?;
        let schema_json: Json = row.try_get("arrow_schema").map_err(sql_err)?;
        let config_json: Json = row.try_get("config").map_err(sql_err)?;

        let schema = json_to_schema(&schema_json)?;
        let config: TableConfig = serde_json::from_value(config_json).unwrap_or_default();
        let current_snapshot_id = current_snapshot_id.ok_or_else(||
            CatalogError::Sql("table has no current snapshot".into()))?;
        let schema_snapshot_id = schema_snapshot_id.ok_or_else(||
            CatalogError::Sql("table has no schema snapshot".into()))?;

        Ok(TableRef {
            id: TableId::from_uuid(id),
            database_id: DatabaseId::from_uuid(database_id),
            name: name.to_owned(),
            current_snapshot_id: SnapshotId::from_uuid(current_snapshot_id),
            schema_snapshot_id: SchemaSnapshotId::from_uuid(schema_snapshot_id),
            schema,
            config,
        })
    }
```

Apply the same recipe to every other `*_in_tenant` method. Each is mechanical — bind tenant first, shift indices, add `tenant_id = $1` predicate, no new logic.

For `create_table_in_tenant`: must verify the database belongs to the same tenant before linking — query `SELECT tenant_id FROM databases WHERE id = $database_id` and reject if mismatched.

For `begin_snapshot_in_tenant`: query `WHERE tenant_id = $1 AND id = $2` on tables, then construct `PgSnapshotTxn::new(pool, tenant, table_id, parent, schema_snap)` (Task 5 adds the tenant field).

For `list_extents_in_tenant`: shift `arg_index` by 1 and add `WHERE tenant_id = $1 AND table_id = $2` as the first two binds; existing prune predicates still build on top.

- [ ] **Step 4: Run isolation test**

Run: `cargo test -p kyma-catalog --test tenant_isolation_it`
Expected: 2 tests PASS.

- [ ] **Step 5: Run full catalog suite**

Run: `cargo test -p kyma-catalog`
Expected: all existing tests pass — they exercise legacy methods which delegate via `DEFAULT_TENANT`.

- [ ] **Step 6: Commit**

```bash
git add crates/kyma-catalog/src/lib.rs crates/kyma-catalog/tests/tenant_isolation_it.rs
git commit -m "feat(catalog): tenant-aware PostgresCatalog + cross-tenant isolation gate test"
```

---

### Task 5: Thread tenant_id through `PgSnapshotTxn`

**Files:**
- Modify: `crates/kyma-catalog/src/snapshot.rs`

- [ ] **Step 1: Add tenant field**

In `crates/kyma-catalog/src/snapshot.rs`:

```rust
use kyma_core::tenant::TenantId;

#[derive(Debug)]
pub struct PgSnapshotTxn {
    pool: PgPool,
    tenant: TenantId,
    table_id: TableId,
    parent_snapshot_id: SnapshotId,
    schema_snapshot_id: SchemaSnapshotId,
    added: Vec<ExtentManifest>,
    removed: Vec<ExtentId>,
}

impl PgSnapshotTxn {
    pub fn new(
        pool: PgPool,
        tenant: TenantId,
        table_id: TableId,
        parent_snapshot_id: SnapshotId,
        schema_snapshot_id: SchemaSnapshotId,
    ) -> Self {
        Self {
            pool,
            tenant,
            table_id,
            parent_snapshot_id,
            schema_snapshot_id,
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
}
```

- [ ] **Step 2: Thread tenant into commit SQL**

Update the `commit` method's `Self {` destructure to pull `tenant`. Then in:

- The snapshot insert — `INSERT INTO snapshots (tenant_id, table_id, parent_id, sequence_number, schema_snapshot_id, summary) VALUES ($1, $2, $3, $4, $5, $6)`. Bind tenant first.
- The manifest insert — `INSERT INTO manifests (tenant_id, snapshot_id, kind, extent_count, byte_size) VALUES ($1, $2, 'data', $3, $4)`. Bind tenant first.
- The extent inserts — add `tenant_id` first column:

```rust
        for e in &added {
            sqlx::query(
                "INSERT INTO extents (
                    tenant_id, id, table_id, manifest_id, schema_snapshot_id, object_path, byte_size,
                    row_count, min_timestamp, max_timestamp, column_stats, present_paths,
                    compaction_gen, created_at
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
            )
            .bind(tenant.as_uuid())
            .bind(e.id.as_uuid())
            .bind(e.table_id.as_uuid())
            .bind(manifest_id)
            .bind(e.schema_snapshot_id.as_uuid())
            .bind(&e.object_path)
            .bind(e.byte_size as i64)
            .bind(e.row_count as i64)
            .bind(e.min_timestamp)
            .bind(e.max_timestamp)
            .bind(&e.column_stats)
            .bind(&e.present_paths)
            .bind(e.compaction_gen as i32)
            .bind(e.created_at)
            .execute(&mut *tx)
            .await
            .map_err(sql_err)?;
        }
```

- The soft-delete UPDATE:

```rust
        if !removed.is_empty() {
            let ids: Vec<Uuid> = removed.iter().map(|e| *e.as_uuid()).collect();
            let now: DateTime<Utc> = Utc::now();
            sqlx::query(
                "UPDATE extents SET deleted_at = $1
                 WHERE tenant_id = $2 AND id = ANY($3) AND deleted_at IS NULL",
            )
            .bind(now)
            .bind(tenant.as_uuid())
            .bind(&ids)
            .execute(&mut *tx)
            .await
            .map_err(sql_err)?;
        }
```

- The CAS UPDATE on `tables`:

```rust
        let swapped = sqlx::query(
            "UPDATE tables SET current_snapshot_id = $1
             WHERE tenant_id = $2 AND id = $3 AND current_snapshot_id = $4",
        )
        .bind(new_snap_id)
        .bind(tenant.as_uuid())
        .bind(table_id.as_uuid())
        .bind(parent_snapshot_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(sql_err)?;
```

Update destructure: `let Self { pool, tenant, table_id, parent_snapshot_id, schema_snapshot_id, added, removed } = *self;`

- [ ] **Step 3: Run catalog suite**

Run: `cargo test -p kyma-catalog`
Expected: PASS — including snapshot CAS conflict test.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-catalog/src/snapshot.rs
git commit -m "feat(catalog): PgSnapshotTxn carries tenant_id through commit"
```

---

### Task 6: Tenant-segmented storage paths in `kyma-format-tlm`

**Files:**
- Modify: `crates/kyma-format-tlm/src/lib.rs`
- Modify: `crates/kyma-format-tlm/src/writer.rs`

Object-store paths become `<KYMA_PATH_PREFIX>/<tenant_id>/extents/<extent_id>.kyma`. NO fallback read path: extents are addressed by the `object_path` stored on the `extents` row at write time. Existing pre-retrofit extents (which lack a tenant segment) continue to be read by their stored `object_path` verbatim. The migration backfills `tenant_id = DEFAULT_TENANT` on existing extent rows but does NOT rewrite their `object_path`. New extents written post-retrofit get the new path.

- [ ] **Step 1: Add `with_tenant` constructor on `TelemetryFormat`**

In `crates/kyma-format-tlm/src/lib.rs`:

```rust
pub struct TelemetryFormat {
    store: Arc<dyn ObjectStore>,
    path_prefix: String,
    tenant_segment: String, // empty for self-hosted legacy mode
}

impl TelemetryFormat {
    pub fn new(store: Arc<dyn ObjectStore>, path_prefix: impl Into<String>) -> Self {
        Self {
            store,
            path_prefix: path_prefix.into(),
            tenant_segment: String::new(),
        }
    }

    /// Build a format that namespaces every new extent under
    /// `<prefix>/<tenant_id>/extents/<extent_id>.kyma`.
    pub fn with_tenant(
        store: Arc<dyn ObjectStore>,
        path_prefix: impl Into<String>,
        tenant: kyma_core::tenant::TenantId,
    ) -> Self {
        Self {
            store,
            path_prefix: path_prefix.into(),
            tenant_segment: tenant.to_string(),
        }
    }

    pub(crate) fn store(&self) -> &Arc<dyn ObjectStore> { &self.store }
    pub(crate) fn path_prefix(&self) -> &str { &self.path_prefix }
    pub(crate) fn tenant_segment(&self) -> &str { &self.tenant_segment }
}
```

- [ ] **Step 2: Update `format_extent_path`**

In `crates/kyma-format-tlm/src/writer.rs`:

```rust
fn format_extent_path(prefix: &str, tenant_segment: &str, extent_id: &ExtentId) -> String {
    let core = if tenant_segment.is_empty() {
        format!("extents/{extent_id}.kyma")
    } else {
        format!("{tenant_segment}/extents/{extent_id}.kyma")
    };
    if prefix.is_empty() { core } else { format!("{prefix}/{core}") }
}
```

Update call site (around line 362):

```rust
        let object_path = format_extent_path(format.path_prefix(), format.tenant_segment(), &extent_id);
```

- [ ] **Step 3: Add unit tests**

Append to `crates/kyma-format-tlm/src/writer.rs`:

```rust
#[cfg(test)]
mod path_tests {
    use super::format_extent_path;
    use kyma_core::types::ExtentId;
    use uuid::Uuid;

    #[test]
    fn legacy_path_when_tenant_empty() {
        let id = ExtentId::from_uuid(Uuid::nil());
        let path = format_extent_path("kyma", "", &id);
        assert_eq!(path, "kyma/extents/00000000-0000-0000-0000-000000000000.kyma");
    }

    #[test]
    fn tenant_segmented_path() {
        let id = ExtentId::from_uuid(Uuid::nil());
        let tenant = "11111111-1111-1111-1111-111111111111";
        let path = format_extent_path("kyma", tenant, &id);
        assert_eq!(path,
            "kyma/11111111-1111-1111-1111-111111111111/extents/00000000-0000-0000-0000-000000000000.kyma");
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p kyma-format-tlm`
Expected: existing + 2 new path tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-format-tlm/src/lib.rs crates/kyma-format-tlm/src/writer.rs
git commit -m "feat(format-tlm): tenant_id segment in extent object paths"
```

---

### Task 7: Refactor `kyma-server` auth into pluggable `AuthBackend`

**Files:**
- Create: `crates/kyma-server/src/auth/mod.rs`
- Create: `crates/kyma-server/src/auth/backend.rs`
- Create: `crates/kyma-server/src/auth/env_backend.rs`
- Create: `crates/kyma-server/src/auth/db_backend.rs`
- Create: `crates/kyma-server/src/auth/middleware.rs`
- Delete: `crates/kyma-server/src/auth.rs` (content moves into `auth/` module)
- Modify: `crates/kyma-server/Cargo.toml`

- [ ] **Step 1: Add `cloud-auth` cargo feature**

In `crates/kyma-server/Cargo.toml`:

```toml
[features]
default = []
cloud-auth = []
web-ui  = ["dep:kyma-web-assets", "dep:tonic-web", "dep:http-body-util"]
test-support = [
    "dep:kyma-format-tlm",
    "dep:object_store",
    "dep:testcontainers",
    "dep:testcontainers-modules",
    "dep:reqwest",
]
```

Add `sha2 = "0.10"` to `[dependencies]` (verify workspace `Cargo.toml` first; if `sha2` exists as workspace dep use `sha2.workspace = true` instead).

- [ ] **Step 2: Create `auth/mod.rs`**

```rust
//! Pluggable bearer-token authentication.
//!
//! Two backends ship today:
//! - `EnvAuthBackend` (default) — preserves `KYMA_AUTH_TOKENS` env-var semantics.
//! - `DbAuthBackend` (cargo feature `cloud-auth`) — reads `api_tokens` rows
//!   from cloud Postgres.

mod backend;
mod env_backend;
mod middleware;

#[cfg(feature = "cloud-auth")]
mod db_backend;

pub use backend::{AuthBackend, AuthError, Principal, Role};
pub use env_backend::EnvAuthBackend;
pub use middleware::{require_role_middleware, AuthLayerState};

#[cfg(feature = "cloud-auth")]
pub use db_backend::DbAuthBackend;

// Backwards-compat re-export for legacy `kyma-bin` callers.
pub use env_backend::EnvAuthBackend as AuthConfig;
```

- [ ] **Step 3: Create `auth/backend.rs`**

```rust
use async_trait::async_trait;
use kyma_core::tenant::TenantId;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub enum Role {
    Read = 0,
    Write = 1,
    Admin = 2,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "read" => Some(Role::Read),
            "write" => Some(Role::Write),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub tenant: TenantId,
    pub role: Role,
    pub subject: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("missing Authorization: Bearer <token>")]
    MissingToken,
    #[error("unknown token")]
    UnknownToken,
    #[error("auth backend error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait AuthBackend: Send + Sync + 'static {
    fn enabled(&self) -> bool;
    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError>;
}
```

- [ ] **Step 4: Create `auth/env_backend.rs`**

```rust
use super::backend::{AuthBackend, AuthError, Principal, Role};
use async_trait::async_trait;
use kyma_core::tenant::DEFAULT_TENANT;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct EnvAuthBackend {
    inner: Arc<EnvInner>,
}

#[derive(Default)]
struct EnvInner {
    tokens: HashMap<String, Role>,
}

impl EnvAuthBackend {
    pub fn from_env() -> Self {
        let raw = std::env::var("KYMA_AUTH_TOKENS").unwrap_or_default();
        Self::from_str(&raw)
    }

    pub fn from_str(raw: &str) -> Self {
        let mut tokens = HashMap::new();
        for pair in raw.split(',') {
            let pair = pair.trim();
            if pair.is_empty() { continue; }
            let Some((tok, role)) = pair.split_once(':') else { continue };
            let tok = tok.trim();
            let Some(role) = Role::parse(role) else { continue };
            if !tok.is_empty() {
                tokens.insert(tok.to_owned(), role);
            }
        }
        Self { inner: Arc::new(EnvInner { tokens }) }
    }
}

#[async_trait]
impl AuthBackend for EnvAuthBackend {
    fn enabled(&self) -> bool { !self.inner.tokens.is_empty() }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        let role = self.inner.tokens.get(token).copied()
            .ok_or(AuthError::UnknownToken)?;
        Ok(Principal { tenant: DEFAULT_TENANT, role, subject: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn parses_token_role_pairs() {
        let b = EnvAuthBackend::from_str("alice:admin, bob:write, carol:read");
        assert!(b.enabled());
        assert_eq!(b.authenticate("alice").await.unwrap().role, Role::Admin);
        assert_eq!(b.authenticate("bob").await.unwrap().role, Role::Write);
        assert_eq!(b.authenticate("carol").await.unwrap().role, Role::Read);
    }

    #[tokio::test]
    async fn empty_disables_auth() {
        let b = EnvAuthBackend::from_str("");
        assert!(!b.enabled());
    }

    #[tokio::test]
    async fn principal_is_default_tenant() {
        let b = EnvAuthBackend::from_str("alice:admin");
        let p = b.authenticate("alice").await.unwrap();
        assert_eq!(p.tenant, kyma_core::tenant::DEFAULT_TENANT);
    }

    #[tokio::test]
    async fn unknown_token_rejected() {
        let b = EnvAuthBackend::from_str("alice:admin");
        let err = b.authenticate("eve").await.unwrap_err();
        assert!(matches!(err, AuthError::UnknownToken));
    }
}
```

- [ ] **Step 5: Create `auth/db_backend.rs`**

```rust
//! Postgres-backed token auth for the cloud control plane.
//!
//! Reads `api_tokens` rows shared with the cloud control plane. The schema
//! contract (created by Slice 2's cloud control plane):
//!
//! ```sql
//! CREATE TABLE api_tokens (
//!     id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
//!     tenant_id     uuid NOT NULL,
//!     token_hash    bytea NOT NULL UNIQUE,  -- SHA-256(presented_token)
//!     scopes        text NOT NULL,           -- comma-separated; "admin" | "write" | "read"
//!     subject       text,                    -- workspace_id or user_id for audit
//!     last_used_at  timestamptz,
//!     revoked_at    timestamptz,
//!     created_at    timestamptz NOT NULL DEFAULT now()
//! );
//! ```

use super::backend::{AuthBackend, AuthError, Principal, Role};
use async_trait::async_trait;
use kyma_core::tenant::TenantId;
use sqlx::{PgPool, Row};

#[derive(Clone)]
pub struct DbAuthBackend {
    pool: PgPool,
}

impl DbAuthBackend {
    pub fn new(pool: PgPool) -> Self { Self { pool } }

    fn hash_token(token: &str) -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(token.as_bytes());
        h.finalize().to_vec()
    }
}

#[async_trait]
impl AuthBackend for DbAuthBackend {
    fn enabled(&self) -> bool { true }

    async fn authenticate(&self, token: &str) -> Result<Principal, AuthError> {
        let hash = Self::hash_token(token);
        let row = sqlx::query(
            "SELECT tenant_id, scopes, subject FROM api_tokens
             WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AuthError::Backend(e.to_string()))?
        .ok_or(AuthError::UnknownToken)?;

        let tenant_uuid: uuid::Uuid = row.try_get("tenant_id")
            .map_err(|e| AuthError::Backend(e.to_string()))?;
        let scopes: String = row.try_get("scopes")
            .map_err(|e| AuthError::Backend(e.to_string()))?;
        let subject: Option<String> = row.try_get("subject")
            .map_err(|e| AuthError::Backend(e.to_string()))?;

        let role = scopes
            .split(',')
            .filter_map(|s| Role::parse(s.trim()))
            .max()
            .ok_or_else(|| AuthError::Backend(
                format!("api_tokens row has no parseable scope: {scopes}")))?;

        let _ = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE token_hash = $1")
            .bind(&hash)
            .execute(&self.pool)
            .await;

        Ok(Principal {
            tenant: TenantId::from_uuid(tenant_uuid),
            role,
            subject,
        })
    }
}
```

- [ ] **Step 6: Create `auth/middleware.rs`**

```rust
use super::backend::{AuthBackend, AuthError, Role};
use axum::{
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthLayerState {
    pub backend: Arc<dyn AuthBackend>,
    pub required: Role,
}

pub async fn require_role_middleware(
    State(state): State<AuthLayerState>,
    mut req: Request,
    next: Next,
) -> Response {
    if !state.backend.enabled() {
        req.extensions_mut().insert(super::backend::Principal {
            tenant: kyma_core::tenant::DEFAULT_TENANT,
            role: Role::Admin,
            subject: None,
        });
        req.extensions_mut().insert(kyma_core::tenant::DEFAULT_TENANT);
        return next.run(req).await;
    }

    let token = req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim);
    let Some(token) = token else {
        return unauthorized("missing Authorization: Bearer <token>");
    };

    let principal = match state.backend.authenticate(token).await {
        Ok(p) => p,
        Err(AuthError::UnknownToken) | Err(AuthError::MissingToken) => {
            return unauthorized("unknown token");
        }
        Err(AuthError::Backend(e)) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("auth backend: {e}"))
                .into_response();
        }
    };

    if principal.role < state.required {
        return forbidden(&format!(
            "token role `{:?}` below required `{:?}`",
            principal.role, state.required));
    }

    let tenant = principal.tenant;
    req.extensions_mut().insert(principal);
    req.extensions_mut().insert(tenant);
    next.run(req).await
}

fn unauthorized(msg: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, r#"Bearer realm="kyma""#)],
        msg.to_owned(),
    ).into_response()
}

fn forbidden(msg: &str) -> Response {
    (StatusCode::FORBIDDEN, msg.to_owned()).into_response()
}
```

- [ ] **Step 7: Delete old `auth.rs`**

```bash
rm crates/kyma-server/src/auth.rs
```

- [ ] **Step 8: Run unit tests**

Run: `cargo test -p kyma-server --lib auth`
Expected: 4 EnvAuthBackend tests PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/kyma-server/Cargo.toml crates/kyma-server/src/auth/
git rm crates/kyma-server/src/auth.rs
git commit -m "refactor(server): pluggable AuthBackend trait + EnvAuthBackend + DbAuthBackend"
```

---

### Task 8: Wire `kyma-bin` to construct an `AuthBackend`

**Files:**
- Modify: `crates/kyma-bin/src/main.rs`
- Modify: `crates/kyma-bin/Cargo.toml`

- [ ] **Step 1: Pick backend at startup**

In `crates/kyma-bin/src/main.rs`, replace the existing `let auth = AuthConfig::from_env();` block (around line 135):

```rust
    use std::sync::Arc;
    use kyma_server::auth::{AuthBackend, AuthLayerState, EnvAuthBackend, Role};
    let backend: Arc<dyn AuthBackend> = match std::env::var("KYMA_AUTH_BACKEND").ok().as_deref() {
        #[cfg(feature = "cloud-auth")]
        Some("db") => {
            use kyma_server::auth::DbAuthBackend;
            Arc::new(DbAuthBackend::new(pg_pool.clone()))
        }
        _ => Arc::new(EnvAuthBackend::from_env()),
    };
    if backend.enabled() {
        info!("auth: bearer-token protection enabled");
    } else {
        info!("auth: disabled (set KYMA_AUTH_TOKENS to enable)");
    }
```

- [ ] **Step 2: Replace every `from_fn_with_state` callsite**

Each existing usage:

```rust
.layer(axum::middleware::from_fn_with_state(
    (auth.clone(), Role::Write),
    require_role_middleware,
))
```

becomes:

```rust
.layer(axum::middleware::from_fn_with_state(
    AuthLayerState { backend: backend.clone(), required: Role::Write },
    kyma_server::auth::require_role_middleware,
))
```

There are six usages around lines 171–173, 192–195, 233–236, 237–239, 240–242, 266–269. Replace each.

Update imports: replace `use kyma_server::auth::{require_role_middleware, AuthConfig, Role};` with `use kyma_server::auth::Role;` (the rest are imported in step 1's block).

- [ ] **Step 3: Add `cloud-auth` feature to `kyma-bin`**

In `crates/kyma-bin/Cargo.toml`:

```toml
[features]
default = []
cloud-auth = ["kyma-server/cloud-auth"]
```

- [ ] **Step 4: Build both flavors**

Run: `cargo build -p kyma-bin`
Expected: builds.
Run: `cargo build -p kyma-bin --features cloud-auth`
Expected: builds with DbAuthBackend.

- [ ] **Step 5: Commit**

```bash
git add crates/kyma-bin/src/main.rs crates/kyma-bin/Cargo.toml
git commit -m "feat(bin): pick AuthBackend (env or db) at startup, wire AuthLayerState"
```

---

### Task 9: AuthBackend integration tests

**Files:**
- Create: `crates/kyma-server/tests/auth_backends_it.rs`
- Modify: `crates/kyma-server/Cargo.toml`

- [ ] **Step 1: Write integration test**

Create `crates/kyma-server/tests/auth_backends_it.rs`:

```rust
//! Slice 0 verification gate: both AuthBackend impls work end-to-end.

#![cfg(all(feature = "test-support", feature = "cloud-auth"))]

use kyma_core::tenant::{TenantId, DEFAULT_TENANT};
use kyma_server::auth::{AuthBackend, DbAuthBackend, EnvAuthBackend, Role};
use sha2::{Digest, Sha256};
use sqlx::postgres::PgPoolOptions;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

#[tokio::test]
async fn env_backend_accepts_known_token_and_returns_default_tenant() {
    let b = EnvAuthBackend::from_str("alpha:admin,beta:read");
    assert!(b.enabled());
    let p = b.authenticate("alpha").await.unwrap();
    assert_eq!(p.role, Role::Admin);
    assert_eq!(p.tenant, DEFAULT_TENANT);
    let p2 = b.authenticate("beta").await.unwrap();
    assert_eq!(p2.role, Role::Read);
    assert_eq!(p2.tenant, DEFAULT_TENANT);
}

#[tokio::test]
async fn env_backend_rejects_unknown_token() {
    let b = EnvAuthBackend::from_str("alpha:admin");
    assert!(b.authenticate("ghost").await.is_err());
}

#[tokio::test]
async fn db_backend_returns_workspace_tenant() {
    let container = Postgres::default()
        .with_user("kyma").with_password("kyma_dev").with_db_name("kyma")
        .start().await.expect("postgres up");
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");
    let pool = PgPoolOptions::new().max_connections(4).connect(&url).await.unwrap();

    sqlx::query(
        r#"
        CREATE EXTENSION IF NOT EXISTS "pgcrypto";
        CREATE TABLE api_tokens (
            id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
            tenant_id     uuid NOT NULL,
            token_hash    bytea NOT NULL UNIQUE,
            scopes        text NOT NULL,
            subject       text,
            last_used_at  timestamptz,
            revoked_at    timestamptz,
            created_at    timestamptz NOT NULL DEFAULT now()
        );
        "#,
    ).execute(&pool).await.unwrap();

    let workspace_a = TenantId::from_uuid(Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap());
    let workspace_b = TenantId::from_uuid(Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap());

    let token_a = "tok-workspace-a-secret";
    let token_b = "tok-workspace-b-secret";
    let hash = |t: &str| {
        let mut h = Sha256::new();
        h.update(t.as_bytes());
        h.finalize().to_vec()
    };

    sqlx::query(
        "INSERT INTO api_tokens (tenant_id, token_hash, scopes, subject) VALUES ($1, $2, $3, $4)",
    )
    .bind(workspace_a.as_uuid()).bind(hash(token_a)).bind("admin").bind("workspace_a")
    .execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO api_tokens (tenant_id, token_hash, scopes, subject) VALUES ($1, $2, $3, $4)",
    )
    .bind(workspace_b.as_uuid()).bind(hash(token_b)).bind("read").bind("workspace_b")
    .execute(&pool).await.unwrap();

    let backend = DbAuthBackend::new(pool);
    assert!(backend.enabled());

    let pa = backend.authenticate(token_a).await.unwrap();
    assert_eq!(pa.tenant, workspace_a);
    assert_eq!(pa.role, Role::Admin);
    assert_eq!(pa.subject.as_deref(), Some("workspace_a"));

    let pb = backend.authenticate(token_b).await.unwrap();
    assert_eq!(pb.tenant, workspace_b);
    assert_eq!(pb.role, Role::Read);

    assert!(backend.authenticate("nonexistent-token").await.is_err());
}

#[tokio::test]
async fn db_backend_rejects_revoked_tokens() {
    let container = Postgres::default()
        .with_user("kyma").with_password("kyma_dev").with_db_name("kyma")
        .start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://kyma:kyma_dev@localhost:{port}/kyma");
    let pool = PgPoolOptions::new().max_connections(4).connect(&url).await.unwrap();

    sqlx::query(
        r#"
        CREATE EXTENSION IF NOT EXISTS "pgcrypto";
        CREATE TABLE api_tokens (
            id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
            tenant_id uuid NOT NULL,
            token_hash bytea NOT NULL UNIQUE,
            scopes text NOT NULL,
            subject text,
            last_used_at timestamptz,
            revoked_at timestamptz,
            created_at timestamptz NOT NULL DEFAULT now()
        );
        "#,
    ).execute(&pool).await.unwrap();

    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let token = "revoked-tok";
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let hash = h.finalize().to_vec();
    sqlx::query(
        "INSERT INTO api_tokens (tenant_id, token_hash, scopes, revoked_at)
         VALUES ($1, $2, 'admin', now())",
    )
    .bind(tenant.as_uuid()).bind(&hash)
    .execute(&pool).await.unwrap();

    let backend = DbAuthBackend::new(pool);
    assert!(backend.authenticate(token).await.is_err());
}
```

- [ ] **Step 2: Wire test gate in Cargo.toml**

Add to `crates/kyma-server/Cargo.toml`:

```toml
[[test]]
name = "auth_backends_it"
required-features = ["test-support", "cloud-auth"]
```

Add `sha2 = "0.10"` to `[dev-dependencies]` if not already there.

- [ ] **Step 3: Run test**

Run: `cargo test -p kyma-server --features test-support,cloud-auth --test auth_backends_it`
Expected: 4 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/kyma-server/tests/auth_backends_it.rs crates/kyma-server/Cargo.toml
git commit -m "test(server): integration tests for EnvAuthBackend + DbAuthBackend"
```

---

### Task 10: Retrofit connector direct-SQL helpers for tenant_id

**Files:**
- Modify: `crates/kyma-connectors/src/catalog_sql.rs`
- Modify: `crates/kyma-connectors/src/admin.rs`

The migration adds `tenant_id` to `connectors`/`connector_cursors`/`connector_leases`, but the Rust direct-SQL helpers don't bind it. After Task 2, INSERTs into `connectors` will fail (NOT NULL violation). This task threads tenant from the request's `Principal` extension through.

- [ ] **Step 1: Read existing helpers**

Read `crates/kyma-connectors/src/catalog_sql.rs` end-to-end. The three load-bearing helpers are `create_connector_direct`, `list_due_periodic`, and `load_connector` (or similar names).

- [ ] **Step 2: Update `create_connector_direct`**

Add `tenant: TenantId` as first parameter. INSERT becomes:

```rust
sqlx::query(
    "INSERT INTO connectors (tenant_id, name, kind, config, ...) VALUES ($1, $2, $3, $4, ...)"
)
.bind(tenant.as_uuid())
.bind(name)
.bind(kind)
.bind(config_json)
// ... rest of binds shifted by 1
```

- [ ] **Step 3: Update `list_due_periodic`**

Add `tenant: TenantId` as first parameter. SELECT becomes:

```rust
"SELECT id, name, kind, config, last_run_at, schedule_secs FROM connectors
 WHERE tenant_id = $1 AND schedule_secs IS NOT NULL
   AND (last_run_at IS NULL OR last_run_at + (schedule_secs || ' seconds')::interval <= now())"
```

Bind tenant first.

- [ ] **Step 4: Update `load_connector`**

Add `tenant: TenantId` as first parameter. SELECT becomes:

```rust
"SELECT ... FROM connectors WHERE tenant_id = $1 AND name = $2"
```

- [ ] **Step 5: Update `connector_cursors` and `connector_leases` helpers**

Any helper that INSERTs/SELECTs/UPDATEs `connector_cursors` or `connector_leases` gets the same treatment: prepend `tenant: TenantId` parameter, bind first, add `WHERE tenant_id = $1` predicate.

- [ ] **Step 6: Update `crates/kyma-connectors/src/admin.rs`**

The admin axum handlers receive a `Request`. Extract the `Principal` (or `TenantId`) request extension and pass to the helpers:

```rust
use kyma_core::tenant::TenantId;

async fn create_connector(
    Extension(tenant): Extension<TenantId>,
    State(state): State<AdminState>,
    Json(req): Json<CreateConnectorRequest>,
) -> Result<Json<Connector>, AdminError> {
    let id = create_connector_direct(&state.pool, tenant, &req.name, /* ... */).await?;
    // ...
}
```

Repeat for `list_connectors`, `get_connector`, `update_connector`, `delete_connector`, `trigger_run`, etc. Every connector admin endpoint is now tenant-scoped.

- [ ] **Step 7: Run connector tests**

Run: `cargo test -p kyma-connectors`
Expected: PASS — existing tests use `DEFAULT_TENANT` via the request extension or test fixture.

- [ ] **Step 8: Commit**

```bash
git add crates/kyma-connectors/src/catalog_sql.rs crates/kyma-connectors/src/admin.rs
git commit -m "feat(connectors): thread tenant_id through direct-SQL helpers and admin handlers"
```

---

### Task 11: Update existing test fixtures for tenant_id

**Files:**
- Modify: `crates/kyma-server/tests/cleanup_http.rs`

Raw SQL inserts in `cleanup_http.rs` (around lines 145, 156, 223) violate `extents.tenant_id NOT NULL` because they don't supply the column.

- [ ] **Step 1: Patch each `INSERT INTO extents`**

For each of the three inserts in `crates/kyma-server/tests/cleanup_http.rs`, prefix `tenant_id` to the column list and bind `kyma_core::tenant::DEFAULT_TENANT.as_uuid()` first:

```rust
sqlx::query(
    "INSERT INTO extents (tenant_id, table_id, schema_snapshot_id, object_path, byte_size, row_count, deleted_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7)",
)
.bind(kyma_core::tenant::DEFAULT_TENANT.as_uuid())
.bind(table_id)
.bind(schema_snap)
.bind(format!("test/{}.kyma", uuid::Uuid::new_v4()))
.bind(1024_i64)
.bind(100_i64)
.bind(Some(chrono::Utc::now()))
.execute(&pool)
.await
.unwrap();
```

Apply to all three callsites. Add `use kyma_core::tenant::DEFAULT_TENANT;` at top if needed.

- [ ] **Step 2: Run server suite**

Run: `cargo test -p kyma-server --features test-support`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/kyma-server/tests/cleanup_http.rs
git commit -m "test(server): supply tenant_id in raw extent inserts"
```

---

### Task 12: Slice 0 verification gate

**Files:** none (running tests)

- [ ] **Step 1: Run isolation gate**

Run: `cargo test -p kyma-catalog --test tenant_isolation_it`
Expected: 2 tests PASS.

- [ ] **Step 2: Run auth backends gate**

Run: `cargo test -p kyma-server --features test-support,cloud-auth --test auth_backends_it`
Expected: 4 tests PASS.

- [ ] **Step 3: Run full workspace regression check**

Run: `cargo test --workspace --features kyma-server/test-support`
Expected: every previously-passing test still passes.

- [ ] **Step 4: Smoke-test self-hosted Docker build**

Run: `docker build -t kyma-slice0-smoke .`
Expected: image builds. Run a quick `docker run` with seeded data; assert the `/health` and `/v1/databases` endpoints work.

- [ ] **Step 5: Final commit**

```bash
git commit --allow-empty -m "chore: Slice 0 verification gate green"
```

If steps 1–4 are green, Slice 0 is shippable. Hand off to `superpowers:finishing-a-development-branch`.

---

## Self-Review

### 1. Spec coverage

| Spec requirement | Plan task |
| --- | --- |
| tenant_id added to 9+ tables | Task 2 (covers all 9 named + 8 additional) |
| Backfill default then drop default | Task 2 |
| Compound (tenant_id, name) uniques | Task 2 (databases, connectors); tables uses compound via database_id chain |
| Catalog reads/writes tenant-scoped | Tasks 3, 4, 5 |
| Object paths get tenant segment | Task 6 |
| Existing extent migrator vs fallback decision | Task 6 (decided: NO migration, NO fallback — stored object_path verbatim) |
| AuthBackend trait + EnvAuthBackend | Task 7 |
| DbAuthBackend behind cloud-auth feature | Task 7 |
| Cross-tenant isolation gate test | Task 4 + Task 12 |
| Connector secrets schema (KMS deferred) | Task 2 |
| Connector helpers retrofitted (gap filled) | Task 10 |

### 2. Placeholder scan

No "TBD", "TODO", "fill in later". Task 4 step 3 deliberately compresses 12 method bodies into a recipe + 2 example bodies; the named methods are enumerated and the recipe is mechanical. An executor reads existing bodies in `crates/kyma-catalog/src/lib.rs` and applies the recipe.

### 3. Type consistency

- `TenantId` newtype — single shape across all sites.
- `DEFAULT_TENANT` — single constant.
- `AuthBackend` trait shape consistent.
- `Principal { tenant, role, subject }` — same shape everywhere.
- `Role { Read, Write, Admin }` — preserved from original.
- Method naming: `*_in_tenant` suffix consistent.
- `AuthLayerState { backend, required }` — same shape in middleware + bin.
