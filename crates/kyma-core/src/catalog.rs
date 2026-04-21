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
use crate::types::{DatabaseId, ExtentId, NodeId, SchemaRef, SchemaSnapshotId, SnapshotId, TableId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
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
}

// -------------------- Extents --------------------

/// An extent as seen by the planner (manifest row from the catalog).
#[derive(Debug, Clone)]
pub struct ExtentManifest {
    pub id: ExtentId,
    pub table_id: TableId,
    pub schema_snapshot_id: SchemaSnapshotId,
    pub object_path: String,
    pub byte_size: u64,
    pub row_count: u64,
    pub min_timestamp: Option<DateTime<Utc>>,
    pub max_timestamp: Option<DateTime<Utc>>,
    pub column_stats: Json,      // keyed by column name; per-column min/max/null_count/distinct_est
    pub present_paths: Vec<String>,
    pub compaction_gen: u32,
    pub created_at: DateTime<Utc>,
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
    Between { low: Json, high: Json },
    InSet(Vec<Json>),
    /// Text-search pruning: the extent must contain *all* of these
    /// tokens in its word-level index (`column_stats.{col}.tokens`).
    /// If the extent's token set is `null` (overflowed), we include it
    /// to preserve correctness.
    ContainsTokens(Vec<String>),
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

// -------------------- Schema-listing types --------------------

/// Lightweight column descriptor for the UI schema tree.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ColumnInfo {
    pub name: String,
    /// `string`, `int`, `long`, `real`, `datetime`, `bool`, `dynamic`.
    pub r#type: String,
    pub nullable: bool,
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

    async fn create_database(&self, name: &str) -> Result<DatabaseId>;

    async fn create_table(
        &self,
        database_id: DatabaseId,
        name: &str,
        schema: SchemaRef,
        config: TableConfig,
    ) -> Result<TableId>;

    async fn lookup_table(&self, database: &str, name: &str) -> Result<TableRef>;

    /// List every table in a database. Returns (name, TableRef) pairs so
    /// callers can populate a DataFusion `SchemaProvider` in one round-trip.
    async fn list_tables_in_database(&self, database: &str) -> Result<Vec<TableRef>>;

    // --- schema-listing (UI) ---

    /// Lightweight schema tree for the UI: list all database names.
    async fn list_databases(&self) -> Result<Vec<String>, CatalogError>;

    /// Lightweight schema tree for the UI: list all table names in a database.
    async fn list_tables(&self, database: &str) -> Result<Vec<String>, CatalogError>;

    /// Lightweight schema tree for the UI: get column descriptors for a table.
    async fn get_table_columns(
        &self,
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

    async fn begin_snapshot(&self, table_id: TableId) -> Result<Box<dyn SnapshotTxn>>;

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
    ) -> Result<Vec<ExtentManifest>>;

    // --- garbage collection ---

    async fn gc_candidates(&self, before: DateTime<Utc>) -> Result<Vec<ExtentId>>;
    async fn delete_extent_rows(&self, extents: &[ExtentId]) -> Result<()>;

    // --- node identity & heartbeat ---

    async fn register_node(&self, info: NodeInfo) -> Result<NodeLease>;
    async fn heartbeat(&self, lease: &NodeLease) -> Result<()>;
    async fn deregister_node(&self, lease: NodeLease) -> Result<()>;

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

    // --- ingest idempotency ledger ---

    /// Look up a previously-applied idempotency key. Returns `None` if never
    /// seen (or if its TTL has expired).
    async fn lookup_idempotency(&self, key: &str) -> Result<Option<IngestLedgerEntry>>;

    /// Record an idempotency key after a successful ingest. Uses
    /// `INSERT ... ON CONFLICT DO NOTHING` — if a concurrent writer raced
    /// and won, returns `Ok(None)`; otherwise the newly-stored entry.
    async fn record_idempotency(
        &self,
        key: &str,
        entry: IngestLedgerEntry,
        ttl: chrono::Duration,
    ) -> Result<Option<IngestLedgerEntry>>;
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
