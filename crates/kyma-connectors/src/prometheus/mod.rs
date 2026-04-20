//! Prometheus `/metrics` scrape connector.

pub mod parser;

use async_trait::async_trait;
use serde::Deserialize;

use crate::types::{ConfigError, Connector, ConnectorCtx, ConnectorError, ConnectorRun};

#[derive(Default, Clone, Debug)]
pub struct PromConnector;

/// Parsed form of the connector's JSON config. Kept private — validation
/// and `run_once` both deserialize internally.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PromConfig {
    pub endpoint: String,
    #[serde(default)]
    pub auth: Option<PromAuth>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    5_000
}

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
    fn type_id(&self) -> &'static str {
        "prometheus"
    }

    fn validate_config(&self, cfg: &serde_json::Value) -> Result<(), ConfigError> {
        // Chosen policy: strict unknown-field rejection (catch operator
        // typos at POST time), serde-driven cross-field auth validation
        // (Bearer requires token_ref, Basic requires username+password_ref),
        // and a 100..=60_000 ms bound on timeout so a misconfigured value
        // can't tie up a runner slot for minutes.
        let parsed: PromConfig =
            serde_json::from_value(cfg.clone()).map_err(|e| ConfigError(e.to_string()))?;
        let ok_scheme =
            parsed.endpoint.starts_with("http://") || parsed.endpoint.starts_with("https://");
        if !ok_scheme {
            return Err(ConfigError(format!(
                "endpoint scheme must be http or https, got {:?}",
                parsed.endpoint
            )));
        }
        if !(100..=60_000).contains(&parsed.timeout_ms) {
            return Err(ConfigError(format!(
                "timeout_ms must be in [100, 60000], got {}",
                parsed.timeout_ms
            )));
        }
        Ok(())
    }

    async fn run_once(
        &self,
        _ctx: &ConnectorCtx,
        _cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        Err(ConnectorError::Permanent(
            "run_once not yet implemented".into(),
        ))
    }
}
