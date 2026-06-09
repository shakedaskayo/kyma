//! Core types for the connector framework.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use uuid::Uuid;

use crate::metrics::ConnectorMetrics;
use crate::secrets::SecretStore;
use kyma_core::credentials::CredentialStore;
use kyma_core::tenant::TenantId;

/// How a connector is driven — periodic tick or long-lived lease.
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
    /// Target table name within the connector's `target_database`.
    pub table: String,
    /// JSON rows — run through JSON→Arrow coercion before ingest.
    pub rows: Vec<serde_json::Value>,
}

/// Optional hint from a connector that the framework should auto-register
/// a property-graph binding after the tables have been ingested.
#[derive(Debug, Clone)]
pub struct GraphHint {
    /// Name of the graph to register (idempotent — "already exists" is
    /// swallowed).
    pub graph_name: String,
    /// Node table name (must match one of the `TableRows` or
    /// `ConnectorRun::rows` target table names).
    pub node_table: String,
    /// Edge table name.
    pub edge_table: String,
}

/// Produced by a single `run_once` invocation.
#[derive(Debug)]
pub struct ConnectorRun {
    /// Legacy single-table path: JSON rows destined for the connector's
    /// configured `target_table`. Kept for backwards-compatibility with
    /// existing connectors (e.g. Prometheus). Set to an empty `Vec` when
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

/// Context passed to `Connector::run_once`. Cheap to clone per-tick; the
/// connector should not retain it beyond the call.
pub struct ConnectorCtx {
    pub connector_id: Uuid,
    /// Tenant the connector belongs to — needed for any tenant-scoped lookup
    /// the run does (credentials, future cross-table reads, …).
    pub tenant: TenantId,
    pub http: reqwest::Client,
    pub secrets: Arc<dyn SecretStore>,
    /// System-wide typed credentials store. Connectors that accept a
    /// `credential_id` in their config resolve it via this trait — see
    /// [`crate::credentials_util`] for the convenience helper.
    pub credentials: Arc<dyn CredentialStore>,
    /// Run-time OAuth capability (pool + crypto) for connectors that resolve a
    /// fresh access token via [`crate::oauth::valid_access_token`]. `None` when
    /// the runner wasn't wired with OAuth (e.g. tests), in which case token
    /// refresh falls back to operator-env client credentials only.
    pub oauth: Option<crate::oauth::OAuthRuntime>,
    /// Tick timestamp (bucketed to the schedule grid). Used for the
    /// idempotency key and as a fallback sample timestamp.
    pub scheduled_for: DateTime<Utc>,
    pub metrics: ConnectorMetrics,
    /// Object-store artifact capability for connectors that persist full-file
    /// blobs (e.g. CI job logs). `None` when the runner wasn't wired with an
    /// artifact store (tests, or connectors that never emit blobs), in which
    /// case the connector must degrade gracefully (skip capture, still emit
    /// metadata rows).
    pub artifacts: Option<Arc<dyn crate::artifacts::ArtifactStore>>,
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

    /// Self-describing catalog metadata (label, category, auth mode, config
    /// fields, brand icon, …) that powers the connectors UI. The default
    /// returns a minimal entry derived from [`Self::type_id`]; connectors
    /// should override it so they appear fully described in the catalog.
    fn catalog(&self) -> crate::catalog::CatalogEntry {
        crate::catalog::CatalogEntry::minimal(self.type_id())
    }

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
