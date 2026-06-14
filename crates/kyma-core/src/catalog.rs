//! The catalog — source of truth for table metadata, schemas, snapshots,
//! and extent manifests.
//!
//! # Iceberg-mirroring metadata model
//!
//! The data model deliberately mirrors Apache Iceberg's hierarchy:
//!
//! ```text
//! table  →  current_snapshot  →  manifest_list  →  manifests  →  data_files
//! ```
//!
//! Slice 1 ships a Postgres-backed implementation. The `Catalog` trait lets
//! future slices swap in Iceberg REST, FoundationDB, Raft, or DynamoDB
//! backends without touching engine code.
//!
//! # Concurrency model
//!
//! Every write (ingest, compaction, retention, schema change) goes through
//! a [`SnapshotTxn`]. Commit is an optimistic CAS on
//! `tables.current_snapshot_id` — concurrent writers race; loser retries.
//! No distributed locks.

use crate::errors::{CatalogError, Result};
use crate::types::{
    DatabaseId, ExtentId, NodeId, SchemaRef, SchemaSnapshotId, SnapshotId, TableId,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::collections::HashMap;

// -------------------- Table / schema --------------------

/// A table resolved from `(database, name)`. Carries the current schema
/// snapshot and table-level config.
#[derive(Debug, Clone)]
pub struct TableRef {
    pub id: TableId,
    pub database_id: DatabaseId,
    pub name: String,
    pub current_snapshot_id: SnapshotId,
    pub schema_snapshot_id: SchemaSnapshotId,
    pub schema: SchemaRef,
    pub config: TableConfig,
}

/// Per-table configuration stored as `tables.config` jsonb.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TableConfig {
    pub retention_days: Option<u32>,
    pub extent_target_bytes: Option<u64>,
    pub wal_replication_interval_ms: Option<u32>,
    pub ingest_max_batch_bytes: Option<u64>,
    pub dead_letter_table: Option<String>,
    /// When `Some`, this table is a **federated** (live-proxied) table: its
    /// schema is cataloged here but its rows live on an external platform and
    /// are fetched at query time. Federated tables hold no extents and reject
    /// ingest. See `kyma-federation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub federated: Option<FederatedTableSpec>,
}

/// Connection + addressing info for one federated table — self-contained so a
/// query path can build a live remote scan from the `TableRef` alone (plus the
/// credential store). Written by federated data sources (e.g. `msfabric`) on
/// every metadata-sync tick.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FederatedTableSpec {
    /// Platform id, e.g. `"msfabric"` (Microsoft Fabric SQL endpoint).
    pub platform: String,
    /// Platform endpoint host, e.g. `<id>.<region>.fabric.microsoft.com`.
    pub endpoint: String,
    /// Remote database/catalog name on the endpoint (Fabric: the lakehouse or
    /// warehouse item name).
    pub remote_database: String,
    /// Remote schema the table lives in (Fabric/T-SQL: usually `dbo`).
    pub remote_schema: String,
    /// Remote table name (unqualified).
    pub remote_table: String,
    /// Credential used to authenticate against the platform.
    pub credential_id: uuid::Uuid,
    /// When true, this table is skipped by the cross-database `x-database: *`
    /// union (live fan-out opt-out); explicit queries still work.
    #[serde(default)]
    pub exclude_from_wildcard: bool,
}

// -------------------- Extents --------------------

/// An extent as seen by the planner (manifest row from the catalog).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtentManifest {
    pub id: ExtentId,
    pub table_id: TableId,
    pub schema_snapshot_id: SchemaSnapshotId,
    pub object_path: String,
    pub byte_size: u64,
    pub row_count: u64,
    pub min_timestamp: Option<DateTime<Utc>>,
    pub max_timestamp: Option<DateTime<Utc>>,
    pub column_stats: Json, // keyed by column name; per-column min/max/null_count/distinct_est
    pub present_paths: Vec<String>,
    pub compaction_gen: u32,
    pub created_at: DateTime<Utc>,
}

/// A staged-but-not-yet-committed extent (S2.2). The router wrote the object and
/// recorded its manifest here; the committer drains these and commits them.
#[derive(Debug, Clone)]
pub struct StagedExtentRow {
    /// `staged_extents.id` — the row to delete on commit.
    pub id: uuid::Uuid,
    pub table_id: TableId,
    pub manifest: ExtentManifest,
}

// -------------------- Pruning --------------------

/// A predicate the catalog can evaluate against manifest-level stats.
///
/// The catalog evaluates this in SQL: `min_timestamp/max_timestamp` via a
/// GIST range index; `present_paths` via GIN; column predicates via JSONB
/// path ops. Queries filter extents _before_ any object-store I/O.
#[derive(Debug, Clone, Default)]
pub struct PrunePredicate {
    pub time_range: Option<TimeRange>,
    pub required_paths: Vec<String>,
    pub column_predicates: HashMap<String, ColumnPrune>,
}

#[derive(Debug, Clone)]
pub struct TimeRange {
    pub start_inclusive: DateTime<Utc>,
    pub end_exclusive: DateTime<Utc>,
}

/// Simple column-level predicate shapes the catalog can evaluate against
/// stored `column_stats`.
#[derive(Debug, Clone)]
pub enum ColumnPrune {
    Equals(Json),
    Between {
        low: Json,
        high: Json,
    },
    InSet(Vec<Json>),
    /// Text-search pruning: the extent must contain *all* of these
    /// tokens in its word-level index (`column_stats.{col}.tokens`).
    /// If the extent's token set is `null` (overflowed), we include it
    /// to preserve correctness.
    ContainsTokens(Vec<String>),
    /// Vector ANN pruning: keep the extent only if its per-extent centroid +
    /// radius give a cosine-distance lower bound below `threshold`. `query` is
    /// the raw query embedding (normalized internally). Evaluated as a Rust
    /// post-filter (no SQL form); extents lacking a `vec` stat are kept
    /// (exact-scan fallback). See [`cosine_distance_lower_bound`].
    VectorDistance {
        query: Vec<f32>,
        threshold: f64,
    },
}

/// Conservative lower bound on cosine distance from `query` to any row in an
/// extent summarized by `centroid` (mean of L2-normalized rows) + `radius`
/// (max L2 distance to centroid on the unit sphere).
///
/// On the unit sphere, cosine distance = ½‖q̂ − r̂‖², and the L2 triangle
/// inequality gives `min_r ‖q̂ − r̂‖ ≥ max(0, ‖q̂ − c‖ − radius)`. Hence the
/// lower bound is `½·max(0, ‖q̂ − c‖ − radius)²` — never an over-estimate, so
/// pruning by it yields no false negatives. Returns `None` if the query is a
/// zero vector or the dimensions mismatch (caller should then keep the extent).
pub fn cosine_distance_lower_bound(query: &[f32], centroid: &[f64], radius: f64) -> Option<f64> {
    if query.len() != centroid.len() || query.is_empty() {
        return None;
    }
    let norm = query
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt();
    if norm <= 1e-12 {
        return None;
    }
    let mut dist_sq = 0.0f64;
    for (q, c) in query.iter().zip(centroid.iter()) {
        let qn = *q as f64 / norm;
        let diff = qn - *c;
        dist_sq += diff * diff;
    }
    let dist = dist_sq.sqrt();
    let d = (dist - radius).max(0.0);
    Some(0.5 * d * d)
}

#[cfg(test)]
mod ann_bound_tests {
    use super::cosine_distance_lower_bound;

    #[test]
    fn collapses_to_zero_when_radius_covers_query() {
        // Centroid coincides with the (normalized) query; any radius ⇒ lb 0.
        let lb = cosine_distance_lower_bound(&[1.0, 0.0, 0.0], &[1.0, 0.0, 0.0], 0.5).unwrap();
        assert!(lb <= 1e-9, "expected ~0, got {lb}");
    }

    #[test]
    fn max_distance_for_opposite_centroid_tight_radius() {
        // q̂ = +x, centroid = −x, radius 0 ⇒ ‖q̂−c‖ = 2 ⇒ lb = ½·2² = 2 (cosine max).
        let lb = cosine_distance_lower_bound(&[1.0, 0.0], &[-1.0, 0.0], 0.0).unwrap();
        assert!((lb - 2.0).abs() < 1e-9, "got {lb}");
    }

    #[test]
    fn query_is_normalized_before_bounding() {
        // Unnormalized query (length 5) must give the same bound as its unit form.
        let a = cosine_distance_lower_bound(&[5.0, 0.0], &[-1.0, 0.0], 0.0).unwrap();
        let b = cosine_distance_lower_bound(&[1.0, 0.0], &[-1.0, 0.0], 0.0).unwrap();
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn unbounded_cases_return_none() {
        assert!(cosine_distance_lower_bound(&[0.0, 0.0], &[1.0, 0.0], 0.0).is_none());
        assert!(cosine_distance_lower_bound(&[1.0, 0.0, 0.0], &[1.0, 0.0], 0.0).is_none());
    }
}

// -------------------- Snapshot transactions --------------------

/// A transactional handle on a snapshot-in-progress.
///
/// `commit` atomically advances `tables.current_snapshot_id` via optimistic
/// CAS on the parent snapshot id — if another writer committed in the
/// meantime, `commit` returns [`CatalogError::Conflict`][crate::errors::CatalogError::Conflict]
/// and the caller is expected to re-read, re-lineage, retry.
#[async_trait]
pub trait SnapshotTxn: Send {
    /// Parent snapshot id this transaction branches from.
    fn parent_snapshot_id(&self) -> SnapshotId;

    /// Register a new extent in the new snapshot.
    async fn add_extent(&mut self, manifest: ExtentManifest) -> Result<()>;

    /// Mark existing extents as removed (compaction / retention).
    async fn remove_extents(&mut self, extents: &[ExtentId]) -> Result<()>;

    /// Commit. Returns the new snapshot id, or `CatalogError::Conflict`.
    async fn commit(self: Box<Self>, summary: SnapshotSummary) -> Result<SnapshotId>;

    /// Abandon the transaction without committing.
    async fn rollback(self: Box<Self>) -> Result<()>;
}

/// Summary metadata captured on every snapshot commit — used by GC,
/// observability, and audit logs.
#[derive(Debug, Clone, Default)]
pub struct SnapshotSummary {
    pub rows_added: i64,
    pub rows_removed: i64,
    pub bytes_added: i64,
    pub bytes_removed: i64,
    /// Free-form tag: `"ingest"`, `"compaction"`, `"retention"`, `"schema_change"`.
    pub operation: String,
}

// -------------------- Node identity --------------------

/// A node's logical role in the cluster. Slice 1 runs exactly one node as
/// `AllInOne`; later slices start splitting.
#[derive(Debug, Clone, Copy, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeRole {
    AllInOne,
    Ingest,
    Query,
    Compaction,
}

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub role: NodeRole,
    pub endpoint: String,
    pub capabilities: Json,
}

/// A heartbeat lease held by a registered node. Renewed via
/// [`Catalog::heartbeat`]; expires if not renewed.
#[derive(Debug, Clone)]
pub struct NodeLease {
    pub node_id: NodeId,
    pub lease_id: uuid::Uuid,
    pub expires_at: DateTime<Utc>,
}

/// A live node returned by [`Catalog::list_live_nodes`]. Minimal view
/// sufficient for the read-router.
#[derive(Debug, Clone)]
pub struct LiveNode {
    pub node_id: NodeId,
    pub role: NodeRole,
    /// Advertised Arrow Flight endpoint (host:port). The read-router
    /// calls back into it with a `kind:"extent"` ticket.
    pub endpoint: String,
    pub last_heartbeat: DateTime<Utc>,
}

// -------------------- Schema-listing types --------------------

/// Lightweight column descriptor for the UI schema tree.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ColumnInfo {
    pub name: String,
    /// `string`, `int`, `long`, `real`, `datetime`, `bool`, `dynamic`.
    pub r#type: String,
    pub nullable: bool,
}

// -------------------- Dashboards --------------------

/// A saved dashboard definition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Dashboard {
    pub id: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub time_range_preset: String,
    pub refresh_interval_seconds: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// A single panel within a dashboard.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardPanel {
    pub id: uuid::Uuid,
    pub dashboard_id: uuid::Uuid,
    pub title: String,
    /// `"chart"` | `"table"` | `"markdown"` | `"stat"`
    pub panel_type: String,
    pub query: Option<String>,
    pub database_name: Option<String>,
    pub config: serde_json::Value,
    pub grid_x: i32,
    pub grid_y: i32,
    pub grid_w: i32,
    pub grid_h: i32,
    pub display_order: i32,
}

/// A dashboard with its panels fully loaded.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DashboardWithPanels {
    #[serde(flatten)]
    pub dashboard: Dashboard,
    pub panels: Vec<DashboardPanel>,
}

/// Patch payload for `update_dashboard`. Every field is optional; absent
/// fields are left unchanged. `panels: Some(vec)` atomically replaces all
/// panels; `panels: None` leaves panels untouched.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct DashboardUpdate {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub time_range_preset: Option<String>,
    pub refresh_interval_seconds: Option<Option<i32>>,
    /// When present, REPLACES all panels for this dashboard atomically.
    /// `None` means leave panels alone.
    pub panels: Option<Vec<DashboardPanelInput>>,
}

/// Input shape for creating or replacing a panel.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DashboardPanelInput {
    /// Optional — if absent a new UUID is generated; if present it must
    /// belong to this dashboard (or will be treated as a new panel id).
    pub id: Option<uuid::Uuid>,
    pub title: String,
    pub panel_type: String,
    pub query: Option<String>,
    pub database_name: Option<String>,
    pub config: serde_json::Value,
    pub grid_x: i32,
    pub grid_y: i32,
    pub grid_w: i32,
    pub grid_h: i32,
    pub display_order: i32,
}

// -------------------- Auth: users + api tokens --------------------

/// A user record (password_hash is deliberately omitted — use
/// [`Catalog::get_user_with_hash_in_tenant`] when verification is required).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct User {
    pub id: uuid::Uuid,
    pub username: String,
    pub role: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Resolved principal returned by a successful [`Catalog::lookup_api_token`].
#[derive(Debug, Clone)]
pub struct TokenPrincipal {
    pub tenant: crate::tenant::TenantId,
    pub role: String,
    pub subject: Option<String>,
}

/// Claim resolved from a valid refresh token — the inputs needed to mint a
/// rotated access+refresh pair within the same login session.
#[derive(Debug, Clone)]
pub struct RefreshClaim {
    pub tenant: crate::tenant::TenantId,
    pub role: String,
    pub subject: Option<String>,
    pub session_id: uuid::Uuid,
}

/// An `api_tokens` row for the token-management surface. The raw token is
/// unrecoverable (only its SHA-256 is stored); `token_hash` doubles as the
/// row identifier for revocation.
#[derive(Debug, Clone)]
pub struct ApiTokenInfo {
    pub token_hash: Vec<u8>,
    pub role: String,
    pub subject: Option<String>,
    pub kind: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked: bool,
}

// -------------------- Graphs --------------------

/// A registered property-graph: binds a node table + edge table in a database,
/// with the column roles that identify nodes/edges. Read by the graph layer's
/// stored-graph provider to serve `/v1/graph/<name>/*`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphRegistration {
    pub id: uuid::Uuid,
    pub database: String,
    pub name: String,
    pub node_table: String,
    pub edge_table: String,
    pub id_col: String,
    pub label_col: String,
    pub src_col: String,
    pub dst_col: String,
    pub type_col: String,
    pub realm_col: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Column-role spec supplied when registering a graph. Defaults match the
/// conventional node/edge schema (see the graph-layer design §3.1).
#[derive(Debug, Clone)]
pub struct GraphSpec {
    pub node_table: String,
    pub edge_table: String,
    pub id_col: String,
    pub label_col: String,
    pub src_col: String,
    pub dst_col: String,
    pub type_col: String,
    pub realm_col: Option<String>,
}

impl GraphSpec {
    /// Conventional defaults: nodes(`id`,`labels`), edges(`src`,`dst`,`type`),
    /// optional `realm`. Caller overrides the column names as needed.
    pub fn with_defaults(node_table: impl Into<String>, edge_table: impl Into<String>) -> Self {
        Self {
            node_table: node_table.into(),
            edge_table: edge_table.into(),
            id_col: "id".into(),
            label_col: "labels".into(),
            src_col: "src".into(),
            dst_col: "dst".into(),
            type_col: "type".into(),
            realm_col: None,
        }
    }
}

// -------------------- Embedding backfill (S1.5) --------------------

/// Per-table embedding declaration: "embed the text in `source_column` into the
/// vector `embedding_column` using `model_id`". One row per
/// `(tenant, table, embedding_column)`. Consulted by the async `embed_backfill`
/// scheduler (which extents need filling) and executor (how to fill them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableEmbedConfig {
    pub table_id: TableId,
    /// Text (Utf8) column whose contents are embedded.
    pub source_column: String,
    /// Vector (`FixedSizeList<Float32, dim>`) column the embeddings are written
    /// into.
    pub embedding_column: String,
    /// Stable embedding-backend id, e.g. `"fastembed/bge-small-en-v1.5"`. Mixing
    /// ids across writes to one column is a correctness bug; the executor
    /// asserts the injected backend's id matches.
    pub model_id: String,
    /// Output dimension of `model_id`.
    pub dim: u16,
    /// When true, the backfill scheduler auto-enqueues jobs for un-embedded
    /// extents of this table. When false, backfill is manual-only.
    pub auto_embed: bool,
}

/// Result returned by [`Catalog::cleanup_soft_deleted_extents`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResult {
    /// Number of extent rows hard-deleted from the catalog.
    pub extents_deleted: u64,
    /// Total row count across deleted extents.
    pub rows_freed: u64,
    /// Total byte size across deleted extents.
    pub bytes_freed: u64,
}

// -------------------- The Catalog trait --------------------

/// The catalog. All durable metadata flows through this trait.
#[async_trait]
pub trait Catalog: Send + Sync {
    /// Downcast escape hatch. Lets specialized subsystems (compaction
    /// scheduler, direct-SQL tooling) reach backend-specific APIs on the
    /// concrete catalog impl without those subsystems being coupled through
    /// the trait.
    fn as_ref_any(&self) -> &(dyn std::any::Any + Send + Sync);

    // --- databases & tables ---

    async fn create_database(&self, name: &str) -> Result<DatabaseId> {
        self.create_database_in_tenant(crate::tenant::DEFAULT_TENANT, name)
            .await
    }

    /// Tenant-scoped variant of [`create_database`].
    async fn create_database_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        name: &str,
    ) -> Result<DatabaseId>;

    /// Resolve a database's id by name, or `None` if it doesn't exist. Lets
    /// callers provision-on-demand (look up, then create if missing) without
    /// reaching into a backend-specific pool.
    async fn lookup_database(&self, name: &str) -> Result<Option<DatabaseId>> {
        self.lookup_database_in_tenant(crate::tenant::DEFAULT_TENANT, name)
            .await
    }

    /// Tenant-scoped variant of [`lookup_database`].
    async fn lookup_database_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        name: &str,
    ) -> Result<Option<DatabaseId>>;

    async fn create_table(
        &self,
        database_id: DatabaseId,
        name: &str,
        schema: SchemaRef,
        config: TableConfig,
    ) -> Result<TableId> {
        self.create_table_in_tenant(
            crate::tenant::DEFAULT_TENANT,
            database_id,
            name,
            schema,
            config,
        )
        .await
    }

    /// Tenant-scoped variant of [`create_table`]. The catalog verifies that
    /// `database_id` belongs to `tenant` before creating the table.
    async fn create_table_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database_id: DatabaseId,
        name: &str,
        schema: SchemaRef,
        config: TableConfig,
    ) -> Result<TableId>;

    async fn lookup_table(&self, database: &str, name: &str) -> Result<TableRef> {
        self.lookup_table_in_tenant(crate::tenant::DEFAULT_TENANT, database, name)
            .await
    }

    /// Tenant-scoped variant of [`lookup_table`].
    async fn lookup_table_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
    ) -> Result<TableRef>;

    /// List every table in a database. Returns (name, TableRef) pairs so
    /// callers can populate a DataFusion `SchemaProvider` in one round-trip.
    async fn list_tables_in_database(&self, database: &str) -> Result<Vec<TableRef>> {
        self.list_tables_in_database_in_tenant(crate::tenant::DEFAULT_TENANT, database)
            .await
    }

    /// Tenant-scoped variant of [`list_tables_in_database`].
    async fn list_tables_in_database_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
    ) -> Result<Vec<TableRef>>;

    /// Drop a table and everything under it (snapshots, extent manifests —
    /// object-store data is left for GC). Returns `true` if a table was
    /// removed. Primary consumer: federated metadata sync, which drops +
    /// recreates a table when the remote schema drifts (federated tables
    /// hold no extents, so this is cheap).
    async fn drop_table(&self, database: &str, name: &str) -> Result<bool> {
        self.drop_table_in_tenant(crate::tenant::DEFAULT_TENANT, database, name)
            .await
    }

    /// Tenant-scoped variant of [`drop_table`].
    async fn drop_table_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
    ) -> Result<bool>;

    // --- schema-listing (UI) ---

    /// Lightweight schema tree for the UI: list all database names.
    async fn list_databases(&self) -> Result<Vec<String>, CatalogError> {
        self.list_databases_in_tenant(crate::tenant::DEFAULT_TENANT)
            .await
    }

    /// Tenant-scoped variant of [`list_databases`].
    async fn list_databases_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
    ) -> Result<Vec<String>, CatalogError>;

    /// Lightweight schema tree for the UI: list all table names in a database.
    async fn list_tables(&self, database: &str) -> Result<Vec<String>, CatalogError> {
        self.list_tables_in_tenant(crate::tenant::DEFAULT_TENANT, database)
            .await
    }

    /// Tenant-scoped variant of [`list_tables`].
    async fn list_tables_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
    ) -> Result<Vec<String>, CatalogError>;

    /// Lightweight schema tree for the UI: get column descriptors for a table.
    async fn get_table_columns(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnInfo>, CatalogError> {
        self.get_table_columns_in_tenant(crate::tenant::DEFAULT_TENANT, database, table)
            .await
    }

    /// Tenant-scoped variant of [`get_table_columns`].
    async fn get_table_columns_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnInfo>, CatalogError>;

    /// Add a column to a table's schema (ALTER TABLE ADD COLUMN).
    ///
    /// Creates a new `schema_snapshot` row with the extended schema and
    /// atomically advances `tables.schema_snapshot_id`. Historical extents
    /// keep their original schema_snapshot_id; reads null-fill the new
    /// column (see `KymaTable::scan` promotion logic).
    ///
    /// The column is always nullable; non-nullable ADD COLUMN requires a
    /// backfill pass which lands in a later slice.
    async fn alter_table_add_column(
        &self,
        table_id: TableId,
        column_name: &str,
        column_type: &str,
    ) -> Result<SchemaSnapshotId>;

    // --- snapshot transactions ---

    async fn begin_snapshot(&self, table_id: TableId) -> Result<Box<dyn SnapshotTxn>> {
        self.begin_snapshot_in_tenant(crate::tenant::DEFAULT_TENANT, table_id)
            .await
    }

    /// Tenant-scoped variant of [`begin_snapshot`].
    async fn begin_snapshot_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
    ) -> Result<Box<dyn SnapshotTxn>>;

    // --- planner support ---

    /// List extents visible at `snapshot`, pruned by `prune`.
    ///
    /// This is the **first level** of the three-level pruning cascade — it
    /// should eliminate 99%+ of extents before any object-store I/O.
    async fn list_extents(
        &self,
        table_id: TableId,
        snapshot: SnapshotId,
        prune: &PrunePredicate,
    ) -> Result<Vec<ExtentManifest>> {
        self.list_extents_in_tenant(crate::tenant::DEFAULT_TENANT, table_id, snapshot, prune)
            .await
    }

    /// Tenant-scoped variant of [`list_extents`].
    async fn list_extents_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
        snapshot: SnapshotId,
        prune: &PrunePredicate,
    ) -> Result<Vec<ExtentManifest>>;

    /// Find tables with enough small live extents to warrant compaction.
    ///
    /// Returns, per qualifying table, the ids of its smallest `max_merge` live
    /// extents — the candidate set the compaction worker should merge. A table
    /// qualifies when it has at least `min_extents` live extents smaller than
    /// `small_bytes`. Only groups with ≥2 extents are returned.
    ///
    /// The default returns nothing (a backend opts in by overriding); the
    /// `compaction` scheduler uses this instead of reaching into a backend's
    /// connection pool, so it works against any catalog (Postgres or SQLite).
    async fn compaction_candidates(
        &self,
        small_bytes: i64,
        min_extents: i64,
        max_merge: i64,
    ) -> Result<Vec<(TableId, Vec<ExtentId>)>> {
        let _ = (small_bytes, min_extents, max_merge);
        Ok(Vec::new())
    }

    // --- index sidecars (extent_indexes) ---

    /// Register (idempotent upsert) one index-sidecar descriptor.
    ///
    /// The upsert key is `(extent_id, column, kind, embedding_model_id)` —
    /// re-registering the same logical sidecar replaces `object_path`,
    /// `byte_size`, and `params` in place rather than inserting a duplicate.
    /// Callers upload the object **before** registering, so a crash between
    /// the two leaves only an orphan object, never a dangling row.
    async fn register_index_sidecar(
        &self,
        tenant: crate::tenant::TenantId,
        desc: &crate::index_sidecar::IndexSidecarDescriptor,
    ) -> Result<()>;

    /// List sidecar descriptors for the given extents of `table_id`,
    /// optionally filtered to one [`SidecarKind`][crate::index_sidecar::SidecarKind].
    /// An empty `extent_ids` slice returns an empty result.
    async fn list_index_sidecars(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
        extent_ids: &[ExtentId],
        kind: Option<crate::index_sidecar::SidecarKind>,
    ) -> Result<Vec<crate::index_sidecar::IndexSidecarDescriptor>>;

    /// Delete every sidecar row attached to the given extents, returning the
    /// `object_path`s of the deleted rows so the caller can GC the objects.
    ///
    /// Note the routine GC path does **not** need this: `extent_indexes` rows
    /// cascade when extent rows are hard-deleted, and the physical-delete
    /// worker collects sidecar object paths before dropping the extent rows.
    async fn delete_index_sidecars_for_extents(
        &self,
        tenant: crate::tenant::TenantId,
        extent_ids: &[ExtentId],
    ) -> Result<Vec<String>>;

    // --- global ANN centroid tree (S1.3, server mode) ---

    /// Upsert the global ANN tree row for `(table_id, column, embedding_model_id)`.
    /// Re-registering replaces `generation`, `extent_fingerprint`, `object_path`,
    /// `byte_size`, and `params` in place. Like sidecars, the blob is uploaded
    /// **before** this call, so a crash in between leaves only an orphan object.
    /// Non-Postgres catalogs (local SQLite) may no-op — local mode keeps
    /// per-extent ANN and never builds a global tree.
    async fn upsert_ann_tree(
        &self,
        tenant: crate::tenant::TenantId,
        desc: &crate::index_sidecar::AnnTreeDescriptor,
    ) -> Result<()> {
        let _ = (tenant, desc);
        Ok(())
    }

    /// Fetch the global ANN tree row for `(table_id, column, embedding_model_id)`,
    /// or `None` if no tree exists (the query path then fans out per-extent).
    async fn get_ann_tree(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
        column: &str,
        embedding_model_id: Option<&str>,
    ) -> Result<Option<crate::index_sidecar::AnnTreeDescriptor>> {
        let _ = (tenant, table_id, column, embedding_model_id);
        Ok(None)
    }

    /// Delete the global ANN tree row for a table+column (all models), returning
    /// the deleted `object_path`s so the caller can GC the blobs.
    async fn delete_ann_tree(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
        column: &str,
    ) -> Result<Vec<String>> {
        let _ = (tenant, table_id, column);
        Ok(Vec::new())
    }

    // --- staged extents (S2.2 router/committer; server mode) ---

    /// Router side: record a freshly-written extent's manifest in `staged_extents`
    /// and ack — the object is already on storage ("S3 is the WAL"), so the
    /// committer can commit it to a snapshot asynchronously. Idempotent on
    /// `batch_id` (a retried request re-stages the same batch as a no-op).
    /// Returns `true` if a row was inserted, `false` if `batch_id` already staged.
    /// Default no-op (non-Postgres): staged ingest is server-mode only.
    async fn stage_extent(
        &self,
        tenant: crate::tenant::TenantId,
        batch_id: uuid::Uuid,
        manifest: &ExtentManifest,
    ) -> Result<bool> {
        let _ = (tenant, batch_id, manifest);
        Ok(false)
    }

    /// Committer side: the oldest staged extents (across all tables), up to
    /// `max`. The caller groups by `table_id` and commits each group.
    async fn list_staged_extents(
        &self,
        tenant: crate::tenant::TenantId,
        max: i64,
    ) -> Result<Vec<StagedExtentRow>> {
        let _ = (tenant, max);
        Ok(Vec::new())
    }

    /// Committer side: commit a group of staged manifests for one table as a
    /// single snapshot AND delete those staged rows — **in one transaction**, so
    /// commit + destage are exactly-once (a crash rolls back both; a racing
    /// committer loses the snapshot `(table,sequence)` unique/CAS and, on retry,
    /// finds the rows already destaged). Returns the new snapshot id, or `None`
    /// if `staged_ids` is empty. `Conflict` on a lost CAS (caller retries).
    async fn commit_staged_group(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
        staged_ids: &[uuid::Uuid],
        manifests: &[ExtentManifest],
    ) -> Result<Option<SnapshotId>> {
        let _ = (tenant, table_id, staged_ids, manifests);
        Ok(None)
    }

    // --- garbage collection ---

    async fn gc_candidates(&self, before: DateTime<Utc>) -> Result<Vec<ExtentId>>;
    async fn delete_extent_rows(&self, extents: &[ExtentId]) -> Result<()>;

    /// Hard-delete soft-deleted extents for a given `(database, table)` that
    /// were soft-deleted before `before`.
    ///
    /// Returns aggregate counts for the caller (HTTP response / audit log).
    async fn cleanup_soft_deleted_extents(
        &self,
        database: &str,
        table: &str,
        before: DateTime<Utc>,
    ) -> Result<CleanupResult> {
        self.cleanup_soft_deleted_extents_in_tenant(
            crate::tenant::DEFAULT_TENANT,
            database,
            table,
            before,
        )
        .await
    }

    /// Tenant-scoped variant of [`cleanup_soft_deleted_extents`].
    async fn cleanup_soft_deleted_extents_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        table: &str,
        before: DateTime<Utc>,
    ) -> Result<CleanupResult>;

    // --- node identity & heartbeat ---

    async fn register_node(&self, info: NodeInfo) -> Result<NodeLease>;
    async fn heartbeat(&self, lease: &NodeLease) -> Result<()>;
    async fn deregister_node(&self, lease: NodeLease) -> Result<()>;

    /// List nodes with a heartbeat within `max_stale_secs`. Used by the
    /// read-router to pick peers for fan-out.
    async fn list_live_nodes(&self, max_stale_secs: u32) -> Result<Vec<LiveNode>>;

    // --- background task queue (compaction, retention, gc, …) ---

    /// Submit a new task for eventual execution by a worker.
    async fn submit_task(
        &self,
        kind: &str,
        table_id: Option<TableId>,
        payload: serde_json::Value,
        priority: i32,
    ) -> Result<uuid::Uuid>;

    /// Atomically claim the next pending task of the given kind.
    ///
    /// Uses `FOR UPDATE SKIP LOCKED` so many workers can pull in parallel
    /// without coordination. Returns `None` if no work is available.
    async fn claim_task(
        &self,
        kind: &str,
        node_id: NodeId,
        lease: chrono::Duration,
    ) -> Result<Option<BackgroundTask>>;

    /// Mark a claimed task complete (status → done).
    async fn complete_task(&self, task_id: uuid::Uuid) -> Result<()>;

    /// Mark a claimed task failed. Requeues for retry if attempt < max_attempts.
    async fn fail_task(&self, task_id: uuid::Uuid, error: &str) -> Result<()>;

    // --- dashboards ---

    async fn create_dashboard(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<Dashboard, CatalogError> {
        self.create_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, name, description)
            .await
    }

    /// Tenant-scoped variant of [`create_dashboard`].
    async fn create_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        name: &str,
        description: Option<&str>,
    ) -> Result<Dashboard, CatalogError>;

    async fn list_dashboards(&self) -> Result<Vec<Dashboard>, CatalogError> {
        self.list_dashboards_in_tenant(crate::tenant::DEFAULT_TENANT)
            .await
    }

    /// Tenant-scoped variant of [`list_dashboards`].
    async fn list_dashboards_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
    ) -> Result<Vec<Dashboard>, CatalogError>;

    async fn get_dashboard(
        &self,
        id: uuid::Uuid,
    ) -> Result<Option<DashboardWithPanels>, CatalogError> {
        self.get_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, id)
            .await
    }

    /// Tenant-scoped variant of [`get_dashboard`].
    async fn get_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        id: uuid::Uuid,
    ) -> Result<Option<DashboardWithPanels>, CatalogError>;

    async fn update_dashboard(
        &self,
        id: uuid::Uuid,
        patch: DashboardUpdate,
    ) -> Result<Dashboard, CatalogError> {
        self.update_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, id, patch)
            .await
    }

    /// Tenant-scoped variant of [`update_dashboard`].
    async fn update_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        id: uuid::Uuid,
        patch: DashboardUpdate,
    ) -> Result<Dashboard, CatalogError>;

    async fn delete_dashboard(&self, id: uuid::Uuid) -> Result<bool, CatalogError> {
        self.delete_dashboard_in_tenant(crate::tenant::DEFAULT_TENANT, id)
            .await
    }

    /// Tenant-scoped variant of [`delete_dashboard`].
    async fn delete_dashboard_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        id: uuid::Uuid,
    ) -> Result<bool, CatalogError>;

    // -------------------- Graphs --------------------

    /// Register a property-graph in `database` under `name`.
    async fn create_graph(
        &self,
        database: &str,
        name: &str,
        spec: GraphSpec,
    ) -> Result<GraphRegistration, CatalogError> {
        self.create_graph_in_tenant(crate::tenant::DEFAULT_TENANT, database, name, spec)
            .await
    }
    async fn create_graph_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
        spec: GraphSpec,
    ) -> Result<GraphRegistration, CatalogError>;

    /// List all graphs registered in `database`.
    async fn list_graphs(&self, database: &str) -> Result<Vec<GraphRegistration>, CatalogError> {
        self.list_graphs_in_tenant(crate::tenant::DEFAULT_TENANT, database)
            .await
    }
    async fn list_graphs_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
    ) -> Result<Vec<GraphRegistration>, CatalogError>;

    /// Look up a single graph by `database` + `name`.
    async fn get_graph(
        &self,
        database: &str,
        name: &str,
    ) -> Result<Option<GraphRegistration>, CatalogError> {
        self.get_graph_in_tenant(crate::tenant::DEFAULT_TENANT, database, name)
            .await
    }
    async fn get_graph_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
    ) -> Result<Option<GraphRegistration>, CatalogError>;

    /// Drop a graph registration (does NOT drop the underlying tables).
    /// Returns true if a registration was removed.
    async fn drop_graph(&self, database: &str, name: &str) -> Result<bool, CatalogError> {
        self.drop_graph_in_tenant(crate::tenant::DEFAULT_TENANT, database, name)
            .await
    }
    async fn drop_graph_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        database: &str,
        name: &str,
    ) -> Result<bool, CatalogError>;

    // --- auth: users ---

    /// Create a new user in the given tenant.
    async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        role: &str,
    ) -> Result<User, CatalogError> {
        self.create_user_in_tenant(crate::tenant::DEFAULT_TENANT, username, password_hash, role)
            .await
    }

    /// Tenant-scoped variant of [`create_user`].
    async fn create_user_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        username: &str,
        password_hash: &str,
        role: &str,
    ) -> Result<User, CatalogError>;

    /// Look up a user by username, returning the public [`User`] struct plus
    /// the stored password hash for verification. Returns `None` if the user
    /// does not exist.
    async fn get_user_with_hash(
        &self,
        username: &str,
    ) -> Result<Option<(User, String)>, CatalogError> {
        self.get_user_with_hash_in_tenant(crate::tenant::DEFAULT_TENANT, username)
            .await
    }

    /// Tenant-scoped variant of [`get_user_with_hash`].
    async fn get_user_with_hash_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        username: &str,
    ) -> Result<Option<(User, String)>, CatalogError>;

    /// Count users in the default tenant (used to detect whether seeding is
    /// needed).
    async fn count_users(&self) -> Result<u64, CatalogError> {
        self.count_users_in_tenant(crate::tenant::DEFAULT_TENANT)
            .await
    }

    /// Tenant-scoped variant of [`count_users`].
    async fn count_users_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
    ) -> Result<u64, CatalogError>;

    /// List all users in the default tenant (admin user management).
    async fn list_users(&self) -> Result<Vec<User>, CatalogError> {
        self.list_users_in_tenant(crate::tenant::DEFAULT_TENANT)
            .await
    }

    /// Tenant-scoped variant of [`list_users`].
    async fn list_users_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
    ) -> Result<Vec<User>, CatalogError>;

    /// Replace a user's password hash. Returns `true` if the user existed.
    async fn set_user_password(
        &self,
        username: &str,
        password_hash: &str,
    ) -> Result<bool, CatalogError> {
        self.set_user_password_in_tenant(crate::tenant::DEFAULT_TENANT, username, password_hash)
            .await
    }

    /// Tenant-scoped variant of [`set_user_password`].
    async fn set_user_password_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        username: &str,
        password_hash: &str,
    ) -> Result<bool, CatalogError>;

    /// Change a user's role. Returns `true` if the user existed.
    async fn set_user_role(&self, username: &str, role: &str) -> Result<bool, CatalogError> {
        self.set_user_role_in_tenant(crate::tenant::DEFAULT_TENANT, username, role)
            .await
    }

    /// Tenant-scoped variant of [`set_user_role`].
    async fn set_user_role_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        username: &str,
        role: &str,
    ) -> Result<bool, CatalogError>;

    /// Delete a user. Returns `true` if a user was removed.
    async fn delete_user(&self, username: &str) -> Result<bool, CatalogError> {
        self.delete_user_in_tenant(crate::tenant::DEFAULT_TENANT, username)
            .await
    }

    /// Tenant-scoped variant of [`delete_user`].
    async fn delete_user_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        username: &str,
    ) -> Result<bool, CatalogError>;

    // --- auth: external identities (JIT provisioning) ---

    /// JIT-provision (or refresh) a user authenticated by an external
    /// identity provider (e.g. Supabase Auth). Keyed by
    /// `(auth_provider, external_id)`: creates the user on first sight,
    /// else updates `username` + `role` in place (role is declarative —
    /// re-resolved by the auth backend on every login). External users get
    /// a sentinel password hash that can never verify, so they cannot
    /// password-login.
    async fn upsert_external_user(
        &self,
        provider: &str,
        external_id: &str,
        username: &str,
        role: &str,
    ) -> Result<User, CatalogError> {
        self.upsert_external_user_in_tenant(
            crate::tenant::DEFAULT_TENANT,
            provider,
            external_id,
            username,
            role,
        )
        .await
    }

    /// Tenant-scoped variant of [`upsert_external_user`].
    async fn upsert_external_user_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        provider: &str,
        external_id: &str,
        username: &str,
        role: &str,
    ) -> Result<User, CatalogError>;

    // --- auth: api tokens ---

    /// Insert an API token row (tenant-scoped).
    async fn insert_api_token(
        &self,
        token_hash: &[u8],
        scopes: &str,
        subject: Option<&str>,
        kind: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), CatalogError> {
        self.insert_api_token_in_tenant(
            crate::tenant::DEFAULT_TENANT,
            token_hash,
            scopes,
            subject,
            kind,
            expires_at,
        )
        .await
    }

    /// Tenant-scoped variant of [`insert_api_token`].
    async fn insert_api_token_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        token_hash: &[u8],
        scopes: &str,
        subject: Option<&str>,
        kind: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), CatalogError>;

    /// Look up an API token globally (token_hash is unique across tenants).
    ///
    /// Returns `None` when the token is unknown, revoked, or expired.
    /// As a side-effect, updates `last_used_at` (fire-and-forget).
    async fn lookup_api_token(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<TokenPrincipal>, CatalogError>;

    /// Revoke an API token. Returns `true` if a row was actually revoked
    /// (i.e. it was previously active), `false` if it was already revoked or
    /// not found.
    async fn revoke_api_token(&self, token_hash: &[u8]) -> Result<bool, CatalogError>;

    /// List API tokens of `kind` (e.g. `'api'`) in the default tenant,
    /// newest first. Includes revoked/expired rows — callers filter.
    async fn list_api_tokens(&self, kind: &str) -> Result<Vec<ApiTokenInfo>, CatalogError> {
        self.list_api_tokens_in_tenant(crate::tenant::DEFAULT_TENANT, kind)
            .await
    }

    /// Tenant-scoped variant of [`list_api_tokens`].
    async fn list_api_tokens_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        kind: &str,
    ) -> Result<Vec<ApiTokenInfo>, CatalogError>;

    /// Insert a session token (an `access` or `refresh` token) tagged with a
    /// `session_id` so the whole login session can be rotated and revoked
    /// together. Uses the default tenant.
    async fn insert_session_token(
        &self,
        token_hash: &[u8],
        scopes: &str,
        subject: Option<&str>,
        kind: &str,
        expires_at: DateTime<Utc>,
        session_id: uuid::Uuid,
    ) -> Result<(), CatalogError>;

    /// Look up a **refresh** token (`kind = 'refresh'`, not revoked, not
    /// expired). Returns the claim needed to mint a rotated pair, or `None`.
    /// Refresh tokens are deliberately *not* returned by [`lookup_api_token`],
    /// so they can never authenticate ordinary API requests.
    async fn lookup_refresh_token(
        &self,
        token_hash: &[u8],
    ) -> Result<Option<RefreshClaim>, CatalogError>;

    /// Revoke the entire login session that `token_hash` belongs to — every
    /// access + refresh token sharing its `session_id` (and the token itself
    /// if it has no session). Returns the number of tokens revoked.
    async fn revoke_session_by_token(&self, token_hash: &[u8]) -> Result<u64, CatalogError>;

    // --- ingest idempotency ledger ---

    /// Look up a previously-applied idempotency key. Returns `None` if never
    /// seen (or if its TTL has expired).
    async fn lookup_idempotency(&self, key: &str) -> Result<Option<IngestLedgerEntry>> {
        self.lookup_idempotency_in_tenant(crate::tenant::DEFAULT_TENANT, key)
            .await
    }

    /// Tenant-scoped variant of [`lookup_idempotency`].
    async fn lookup_idempotency_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        key: &str,
    ) -> Result<Option<IngestLedgerEntry>>;

    /// Record an idempotency key after a successful ingest. Uses
    /// `INSERT ... ON CONFLICT DO NOTHING` — if a concurrent writer raced
    /// and won, returns `Ok(None)`; otherwise the newly-stored entry.
    async fn record_idempotency(
        &self,
        key: &str,
        entry: IngestLedgerEntry,
        ttl: chrono::Duration,
    ) -> Result<Option<IngestLedgerEntry>> {
        self.record_idempotency_in_tenant(crate::tenant::DEFAULT_TENANT, key, entry, ttl)
            .await
    }

    /// Tenant-scoped variant of [`record_idempotency`].
    async fn record_idempotency_in_tenant(
        &self,
        tenant: crate::tenant::TenantId,
        key: &str,
        entry: IngestLedgerEntry,
        ttl: chrono::Duration,
    ) -> Result<Option<IngestLedgerEntry>>;

    // --- embedding backfill (S1.5): per-table config + content-hash cache ---

    /// Upsert a per-table embedding config (idempotent on
    /// `(tenant, table_id, embedding_column)`).
    async fn set_table_embed_config(
        &self,
        tenant: crate::tenant::TenantId,
        cfg: &TableEmbedConfig,
    ) -> Result<()>;

    /// List the embedding configs declared for `table_id` (one per
    /// `embedding_column`).
    async fn list_table_embed_configs(
        &self,
        tenant: crate::tenant::TenantId,
        table_id: TableId,
    ) -> Result<Vec<TableEmbedConfig>>;

    /// Batch-lookup cached embeddings by content hash for one `model_id`.
    /// Returns only the hashes that hit; misses are simply absent. The stored
    /// blob is `dim*4` little-endian f32 bytes, deserialized back to `Vec<f32>`.
    async fn get_cached_embeddings(
        &self,
        tenant: crate::tenant::TenantId,
        model_id: &str,
        hashes: &[String],
    ) -> Result<HashMap<String, Vec<f32>>>;

    /// Batch-upsert cached embeddings for one `model_id`. Each entry is
    /// `(content_hash, vector)`; vectors are serialized to little-endian f32
    /// bytes. Re-inserting an existing key is a no-op (the model is
    /// deterministic, so the vector is identical).
    async fn put_cached_embeddings(
        &self,
        tenant: crate::tenant::TenantId,
        model_id: &str,
        dim: u16,
        entries: &[(String, Vec<f32>)],
    ) -> Result<()>;
}

/// Serialize an f32 vector to little-endian bytes (`len*4` bytes). The on-disk
/// form of an embedding in [`Catalog`]'s embedding cache.
pub fn embedding_to_le_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Inverse of [`embedding_to_le_bytes`]: parse little-endian f32 bytes back to a
/// vector. Returns `None` if `bytes` is not a whole number of f32s.
pub fn embedding_from_le_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

#[cfg(test)]
mod embedding_codec_tests {
    use super::{embedding_from_le_bytes, embedding_to_le_bytes};

    #[test]
    fn f32_le_roundtrip() {
        let v = vec![0.0f32, 1.0, -1.5, 3.14159, f32::MIN_POSITIVE, 12345.678];
        let bytes = embedding_to_le_bytes(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        let back = embedding_from_le_bytes(&bytes).expect("whole f32s");
        assert_eq!(back, v);
    }

    #[test]
    fn empty_roundtrips() {
        assert_eq!(embedding_to_le_bytes(&[]), Vec::<u8>::new());
        assert_eq!(embedding_from_le_bytes(&[]), Some(Vec::new()));
    }

    #[test]
    fn ragged_bytes_rejected() {
        assert!(embedding_from_le_bytes(&[0, 1, 2]).is_none());
    }
}

/// A claimed task, ready for execution.
#[derive(Debug, Clone)]
pub struct BackgroundTask {
    pub id: uuid::Uuid,
    pub kind: String,
    pub table_id: Option<TableId>,
    pub payload: serde_json::Value,
    pub priority: i32,
    pub attempt: i32,
    pub max_attempts: i32,
    pub claim_expires_at: DateTime<Utc>,
}

/// Cached record of a previous ingest, keyed by an idempotency header.
#[derive(Debug, Clone)]
pub struct IngestLedgerEntry {
    pub table_id: TableId,
    pub snapshot_id: SnapshotId,
    pub rows_ingested: u64,
    pub bytes_written: u64,
    pub applied_at: DateTime<Utc>,
}
