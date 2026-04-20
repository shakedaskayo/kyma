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
