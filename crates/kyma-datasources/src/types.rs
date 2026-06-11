//! Core types for the data source framework.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::metrics::DataSourceMetrics;
use crate::secrets::SecretStore;
use kyma_core::credentials::CredentialStore;
use kyma_core::tenant::TenantId;

/// How a data source is driven — periodic tick or long-lived lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DriveModel {
    Periodic { interval_ms: u64 },
    Continuous { heartbeat_ms: u64 },
}

/// A named batch of rows destined for a specific table. Used by the
/// multi-table output path so a single `run_once` can populate several
/// tables (e.g. graph nodes + edges) in one tick.
#[derive(Debug)]
pub struct TableRows {
    /// Target table name within the data source's `target_database`.
    pub table: String,
    /// JSON rows — run through JSON→Arrow coercion before ingest.
    pub rows: Vec<serde_json::Value>,
}

/// Optional hint from a data source that the framework should auto-register
/// a property-graph binding after the tables have been ingested.
#[derive(Debug, Clone)]
pub struct GraphHint {
    /// Name of the graph to register (idempotent — "already exists" is
    /// swallowed).
    pub graph_name: String,
    /// Node table name (must match one of the `TableRows` or
    /// `DataSourceRun::rows` target table names).
    pub node_table: String,
    /// Edge table name.
    pub edge_table: String,
}

/// Produced by a single `run_once` invocation.
#[derive(Debug)]
pub struct DataSourceRun {
    /// Legacy single-table path: JSON rows destined for the data source's
    /// configured `target_table`. Kept for backwards-compatibility with
    /// existing data sources (e.g. Prometheus). Set to an empty `Vec` when
    /// using the multi-table `tables` path.
    pub rows: Vec<serde_json::Value>,
    /// When `Some`, the framework upserts this into `connector_cursors`.
    pub new_cursor: Option<serde_json::Value>,
    /// Multi-table output path. Each entry names a distinct target table
    /// and its rows. The framework ingests each entry independently, with
    /// per-table idempotency keys so entries don't deduplicate each other.
    pub tables: Vec<TableRows>,
    /// When `Some`, the framework calls `GraphRegisterFn` after all
    /// tables have been successfully ingested.
    pub graph: Option<GraphHint>,
}

/// Context passed to `DataSource::run_once`. Cheap to clone per-tick; the
/// data source should not retain it beyond the call.
pub struct DataSourceCtx {
    pub data_source_id: Uuid,
    /// Tenant the data source belongs to — needed for any tenant-scoped lookup
    /// the run does (credentials, future cross-table reads, …).
    pub tenant: TenantId,
    pub http: reqwest::Client,
    pub secrets: Arc<dyn SecretStore>,
    /// System-wide typed credentials store. Data sources that accept a
    /// `credential_id` in their config resolve it via this trait — see
    /// [`crate::credentials_util`] for the convenience helper.
    pub credentials: Arc<dyn CredentialStore>,
    /// Run-time OAuth capability (pool + crypto) for data sources that resolve a
    /// fresh access token via [`crate::oauth::valid_access_token`]. `None` when
    /// the runner wasn't wired with OAuth (e.g. tests), in which case token
    /// refresh falls back to operator-env client credentials only.
    pub oauth: Option<crate::oauth::OAuthRuntime>,
    /// Tick timestamp (bucketed to the schedule grid). Used for the
    /// idempotency key and as a fallback sample timestamp.
    pub scheduled_for: DateTime<Utc>,
    pub metrics: DataSourceMetrics,
    /// Object-store artifact capability for data sources that persist full-file
    /// blobs (e.g. CI job logs). `None` when the runner wasn't wired with an
    /// artifact store (tests, or data sources that never emit blobs), in which
    /// case the data source must degrade gracefully (skip capture, still emit
    /// metadata rows).
    pub artifacts: Option<Arc<dyn crate::artifacts::ArtifactStore>>,
    /// Direct catalog access for *metadata-sync* data sources (federated
    /// platforms like `msfabric`) that register schema-only tables instead of
    /// returning rows. `None` when the runner wasn't wired with a catalog —
    /// such data sources must fail with a Config error in that case.
    pub catalog: Option<Arc<dyn kyma_core::catalog::Catalog>>,
    /// The data source's configured target database. Row-producing data sources
    /// never need it (the runner sinks into it on their behalf); metadata-sync
    /// data sources use it as the kyma database to register tables under.
    pub target_database: String,
}

/// Failure classification that determines framework behaviour.
#[derive(Debug, thiserror::Error)]
pub enum DataSourceError {
    /// Retried via `background_tasks` on the next tick.
    #[error("transient: {0}")]
    Transient(String),
    /// Not retried; logged; next tick proceeds on its schedule.
    #[error("permanent: {0}")]
    Permanent(String),
    /// DataSource is disabled with `disabled_reason`; operator must re-enable.
    #[error("config: {0}")]
    Config(String),
}

#[derive(Debug, thiserror::Error)]
#[error("invalid config: {0}")]
pub struct ConfigError(pub String);

/// Implement this trait to add a new data source type.
///
/// Slice-1 registers instances at compile time via [`DataSourceRegistry`].
#[async_trait]
pub trait DataSource: Send + Sync + 'static {
    /// Stable identifier for this data source type (e.g., `"prometheus"`).
    fn type_id(&self) -> &'static str;

    /// Self-describing catalog metadata (label, category, auth mode, config
    /// fields, brand icon, …) that powers the data sources UI. The default
    /// returns a minimal entry derived from [`Self::type_id`]; data sources
    /// should override it so they appear fully described in the catalog.
    fn catalog(&self) -> crate::catalog::CatalogEntry {
        crate::catalog::CatalogEntry::minimal(self.type_id())
    }

    /// Called on `POST`/`PATCH /v1/data sources` before the row is persisted.
    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError>;

    /// Execute one tick. Return rows + optional cursor update.
    async fn run_once(
        &self,
        ctx: &DataSourceCtx,
        cfg: &serde_json::Value,
        cursor: Option<&serde_json::Value>,
    ) -> Result<DataSourceRun, DataSourceError>;
}
