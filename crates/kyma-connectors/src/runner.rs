//! Connector tick runner.

use crate::catalog_sql;
use crate::metrics::{ConnectorMetrics, TickResult};
use crate::registry::ConnectorRegistry;
use crate::secrets::SecretStore;
use crate::types::{ConnectorCtx, ConnectorError, ConnectorRun};
use chrono::Utc;
use futures::future::BoxFuture;
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::{Catalog, NodeInfo, NodeRole};
use kyma_core::types::NodeId;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Thin async "send these rows into WritePath" closure. Passed in so the
/// runner does not need a generic on WritePath; that plumbing lives in
/// kyma-bin where the concrete types are assembled.
pub type RowSink = Arc<
    dyn Fn(
            String,
            String,
            Vec<serde_json::Value>,
            Option<String>,
        ) -> BoxFuture<'static, Result<(), anyhow::Error>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct ConnectorRunner {
    catalog: Arc<PostgresCatalog>,
    registry: Arc<ConnectorRegistry>,
    sink: RowSink,
    secrets: Arc<dyn SecretStore>,
    node_id: String,
    /// Resolved NodeId after the first registration; lazily initialised.
    registered_node: Arc<OnceCell<NodeId>>,
    pub idle_sleep: Duration,
    pub claim_lease: chrono::Duration,
}

impl ConnectorRunner {
    pub fn new<S: SecretStore + 'static>(
        catalog: Arc<PostgresCatalog>,
        registry: Arc<ConnectorRegistry>,
        sink: RowSink,
        secrets: S,
        node_id: String,
    ) -> Self {
        Self {
            catalog,
            registry,
            sink,
            secrets: Arc::new(secrets),
            node_id,
            registered_node: Arc::new(OnceCell::new()),
            idle_sleep: Duration::from_millis(200),
            claim_lease: chrono::Duration::seconds(60),
        }
    }

    /// Ensure this runner has a row in `nodes` and return its `NodeId`.
    /// Registers once; subsequent calls return the cached value cheaply.
    async fn node_id(&self) -> Result<NodeId, anyhow::Error> {
        self.registered_node
            .get_or_try_init(|| async {
                let lease = self
                    .catalog
                    .register_node(NodeInfo {
                        role: NodeRole::Ingest,
                        endpoint: format!("connector-runner:{}", self.node_id),
                        capabilities: serde_json::json!({"connector_runner": true}),
                    })
                    .await?;
                Ok::<NodeId, anyhow::Error>(lease.node_id)
            })
            .await
            .copied()
    }

    pub async fn claim_and_run_one(&self) -> Result<bool, anyhow::Error> {
        let node_id = self.node_id().await?;

        let Some(task) = self
            .catalog
            .claim_task("connector_tick", node_id, self.claim_lease)
            .await?
        else {
            return Ok(false);
        };

        let connector_id = task
            .payload
            .get("connector_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("task missing connector_id"))?;
        let scheduled_for_ms = task
            .payload
            .get("scheduled_for")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| anyhow::anyhow!("task missing scheduled_for"))?;
        let scheduled_for = chrono::DateTime::<Utc>::from_timestamp_millis(scheduled_for_ms)
            .ok_or_else(|| anyhow::anyhow!("bad scheduled_for"))?;

        let conn = match catalog_sql::load_connector(self.catalog.pool(), connector_id).await? {
            Some(c) if c.enabled => c,
            Some(_) => {
                debug!(connector_id = %connector_id, "skipping disabled connector");
                self.catalog.complete_task(task.id).await?;
                return Ok(true);
            }
            None => {
                warn!(connector_id = %connector_id, "connector row missing; completing task");
                self.catalog.complete_task(task.id).await?;
                return Ok(true);
            }
        };

        let impl_arc = self
            .registry
            .lookup(&conn.type_id)
            .ok_or_else(|| anyhow::anyhow!("no registered impl for type {}", conn.type_id))?;

        let cursor = catalog_sql::load_cursor(self.catalog.pool(), connector_id).await?;
        let metrics = ConnectorMetrics {
            connector_id,
            type_id: impl_arc.type_id(),
        };
        let ctx = ConnectorCtx {
            connector_id,
            http: reqwest::Client::builder().build()?,
            secrets: self.secrets.clone(),
            scheduled_for,
            metrics: metrics.clone(),
        };

        let t0 = std::time::Instant::now();
        let outcome = impl_arc
            .run_once(&ctx, &conn.config_jsonb, cursor.as_ref())
            .await;

        match outcome {
            Ok(ConnectorRun { rows, new_cursor }) => {
                let n_rows = rows.len() as u64;
                let idem = format!(
                    "connector:{}:{}",
                    connector_id,
                    scheduled_for_ms * 1_000_000
                );
                (self.sink)(
                    conn.target_database.clone(),
                    conn.target_table.clone(),
                    rows,
                    Some(idem),
                )
                .await?;
                if let Some(c) = new_cursor {
                    catalog_sql::upsert_cursor(self.catalog.pool(), connector_id, &c).await?;
                }
                catalog_sql::mark_run_success(self.catalog.pool(), connector_id, n_rows as i64)
                    .await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_rows(n_rows);
                metrics.record_tick(TickResult::Ok, t0.elapsed().as_secs_f64());
                metrics.set_last_success(Utc::now());
                info!(connector_id = %connector_id, rows = n_rows, "tick ok");
                Ok(true)
            }
            Err(ConnectorError::Transient(msg)) => {
                warn!(connector_id = %connector_id, error = %msg, "transient");
                catalog_sql::mark_run_failure(self.catalog.pool(), connector_id, &msg).await?;
                self.catalog.fail_task(task.id, &msg).await?;
                metrics.record_error("transient");
                metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                Ok(true)
            }
            Err(ConnectorError::Permanent(msg)) => {
                warn!(connector_id = %connector_id, error = %msg, "permanent");
                catalog_sql::mark_run_failure(self.catalog.pool(), connector_id, &msg).await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_error("permanent");
                metrics.record_tick(TickResult::Permanent, t0.elapsed().as_secs_f64());
                Ok(true)
            }
            Err(ConnectorError::Config(msg)) => {
                error!(connector_id = %connector_id, error = %msg, "config");
                catalog_sql::disable_connector(self.catalog.pool(), connector_id, &msg).await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_error("config");
                metrics.record_tick(TickResult::Config, t0.elapsed().as_secs_f64());
                Ok(true)
            }
        }
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(node_id = %self.node_id, "connector runner starting");
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => { info!("connector runner shutdown"); return; }
                res = self.claim_and_run_one() => match res {
                    Ok(true)  => {}
                    Ok(false) => tokio::time::sleep(self.idle_sleep).await,
                    Err(e) => {
                        error!(error = %e, "runner error");
                        tokio::time::sleep(self.idle_sleep).await;
                    }
                }
            }
        }
    }
}
