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
pub enum TickResult {
    Ok,
    Transient,
    Permanent,
    Config,
}

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
