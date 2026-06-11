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

    fn catalog(&self) -> crate::catalog::CatalogEntry {
        use crate::catalog::{CatalogEntry, CatalogField};
        CatalogEntry {
            type_id: "prometheus".into(),
            label: "Prometheus".into(),
            category: "data".into(),
            description: "Scrape metrics from a Prometheus-compatible endpoint into a \
                          time-series table."
                .into(),
            brand: "prometheus".into(),
            auth_mode: "url".into(),
            status: "available".into(),
            default_schedule_ms: 60_000,
            fields: vec![CatalogField::text(
                "endpoint",
                "Prometheus endpoint",
                "http://localhost:9090",
            )],
            resource: None,
            default_target_table: Some("metrics".into()),
            config_defaults: None,
            graph_name: None,
            // Prometheus scrapes a public endpoint URL; if/when bearer auth is
            // added it'll opt into `["pat"]` here.
            accepted_credential_kinds: Vec::new(),
        }
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
        ctx: &ConnectorCtx,
        cfg: &serde_json::Value,
        _cursor: Option<&serde_json::Value>,
    ) -> Result<ConnectorRun, ConnectorError> {
        let parsed: PromConfig = serde_json::from_value(cfg.clone())
            .map_err(|e| ConnectorError::Config(format!("config parse: {e}")))?;

        let (bearer, basic) = match &parsed.auth {
            None | Some(PromAuth::None) => (None, None),
            Some(PromAuth::Bearer { token_ref }) => {
                let t = ctx
                    .secrets
                    .resolve(token_ref)
                    .map_err(|e| ConnectorError::Config(format!("token resolve: {e}")))?;
                (Some(t), None)
            }
            Some(PromAuth::Basic {
                username,
                password_ref,
            }) => {
                let p = ctx
                    .secrets
                    .resolve(password_ref)
                    .map_err(|e| ConnectorError::Config(format!("password resolve: {e}")))?;
                (None, Some((username.clone(), p)))
            }
        };

        let mut attempt: u32 = 0;
        let body = loop {
            let mut req = ctx
                .http
                .get(&parsed.endpoint)
                .timeout(std::time::Duration::from_millis(parsed.timeout_ms))
                .header(
                    "Accept",
                    "application/openmetrics-text; version=1.0.0, text/plain; version=0.0.4",
                );
            if let Some(t) = &bearer {
                req = req.bearer_auth(t);
            }
            if let Some((u, p)) = &basic {
                req = req.basic_auth(u, Some(p));
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        break resp
                            .text()
                            .await
                            .map_err(|e| ConnectorError::Transient(format!("body read: {e}")))?;
                    }
                    if status.as_u16() == 429 || status.is_server_error() {
                        if attempt >= 3 {
                            return Err(ConnectorError::Transient(format!(
                                "HTTP {status} after {attempt} retries"
                            )));
                        }
                    } else {
                        return Err(ConnectorError::Permanent(format!("HTTP {status}")));
                    }
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    if attempt >= 3 {
                        return Err(ConnectorError::Transient(format!(
                            "network: {e} after {attempt} retries"
                        )));
                    }
                }
                Err(e) => return Err(ConnectorError::Permanent(format!("fetch: {e}"))),
            }
            attempt += 1;
            let base_ms = 100u64 * (1u64 << (attempt - 1).min(5));
            let jitter = fastrand::u64(..base_ms / 3 + 1);
            tokio::time::sleep(std::time::Duration::from_millis(
                base_ms.saturating_add(jitter),
            ))
            .await;
        };

        let samples = parser::parse_openmetrics(&body)
            .map_err(|e| ConnectorError::Permanent(format!("parse: {e}")))?;
        let ts = ctx.scheduled_for.to_rfc3339();
        let rows = samples
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "timestamp": ts,
                    "name": s.name,
                    "value": s.value,
                    "labels": s.labels,
                })
            })
            .collect();

        Ok(ConnectorRun {
            rows,
            new_cursor: None,
            tables: vec![],
            graph: None,
        })
    }
}
