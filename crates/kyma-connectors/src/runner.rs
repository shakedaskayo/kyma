//! Connector tick runner.

use crate::catalog_sql;
use crate::metrics::{ConnectorMetrics, TickResult};
use crate::registry::ConnectorRegistry;
use crate::secrets::SecretStore;
use kyma_core::credentials::CredentialStore;
use crate::types::{ConnectorCtx, ConnectorError, ConnectorRun, GraphHint};
use chrono::Utc;
use futures::future::BoxFuture;
use kyma_catalog::PostgresCatalog;
use kyma_core::catalog::Catalog;
use kyma_core::tenant::{TenantId, DEFAULT_TENANT};
use kyma_core::types::NodeId;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
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

/// Async closure that registers (or idempotently re-registers) a
/// property-graph in the catalog. Called after all tables have been
/// successfully ingested. Failures are treated as Transient.
///
/// Arguments: `(database: String, hint: GraphHint)`.
pub type GraphRegisterFn = Arc<
    dyn Fn(String, GraphHint) -> BoxFuture<'static, Result<(), anyhow::Error>> + Send + Sync,
>;

#[derive(Clone)]
pub struct ConnectorRunner {
    catalog: Arc<PostgresCatalog>,
    registry: Arc<ConnectorRegistry>,
    sink: RowSink,
    graph_register: GraphRegisterFn,
    secrets: Arc<dyn SecretStore>,
    /// System-wide credential store, passed into every `ConnectorCtx`.
    /// A no-op stub is supplied by default so existing builds (and tests)
    /// don't need wiring; callers attach the real store via
    /// [`Self::with_credentials`].
    credentials: Arc<dyn CredentialStore>,
    /// Run-time OAuth handle (pool + crypto) for token refresh of OAuth
    /// connectors. Attached via [`Self::with_oauth`]; `None` leaves refresh on
    /// operator-env client creds only.
    oauth: Option<crate::oauth::OAuthRuntime>,
    /// NodeId of the already-registered node (supplied by the caller).
    node_id: NodeId,
    pub idle_sleep: Duration,
    pub claim_lease: chrono::Duration,
}

impl ConnectorRunner {
    pub fn new<S: SecretStore + 'static>(
        catalog: Arc<PostgresCatalog>,
        registry: Arc<ConnectorRegistry>,
        sink: RowSink,
        secrets: S,
        node_id: NodeId,
    ) -> Self {
        // Default graph_register is a no-op; callers can supply a real one
        // via `with_graph_register`.
        let graph_register: GraphRegisterFn =
            Arc::new(|_db, _hint| Box::pin(async move { Ok(()) }));
        // No-op credentials store by default — connectors that don't look up
        // credentials work unchanged; the real store is attached via
        // `with_credentials` from kyma-bin.
        let credentials: Arc<dyn CredentialStore> = Arc::new(NoopCredentialStore);
        Self {
            catalog,
            registry,
            sink,
            graph_register,
            secrets: Arc::new(secrets),
            credentials,
            oauth: None,
            node_id,
            idle_sleep: Duration::from_millis(200),
            // 5 min: long enough for a sizeable repo clone or a multi-page
            // GitLab walk to finish on a normal tick without the lease
            // expiring mid-flight and inviting a concurrent re-claim by
            // another worker. Tasks past their lease are auto-recoverable
            // via the `claim_task` query's stale-claim branch.
            claim_lease: chrono::Duration::seconds(300),
        }
    }

    /// Attach the system-wide credentials store. Without this call the runner
    /// uses a no-op store that errors `not found` on every lookup — fine for
    /// connectors that take inline secrets, required for ones that resolve a
    /// `credential_id`.
    pub fn with_credentials(mut self, store: Arc<dyn CredentialStore>) -> Self {
        self.credentials = store;
        self
    }

    /// Attach the run-time OAuth handle so OAuth connectors can refresh + persist
    /// expired access tokens (resolving per-tenant bring-your-own client creds
    /// from `oauth_clients` in addition to operator env).
    pub fn with_oauth(
        mut self,
        pool: sqlx::PgPool,
        crypto: Arc<kyma_core::crypto::Crypto>,
    ) -> Self {
        self.oauth = Some(crate::oauth::OAuthRuntime { pool, crypto });
        self
    }

    /// Replace the graph-registration closure. Returns `self` for chaining.
    pub fn with_graph_register(mut self, f: GraphRegisterFn) -> Self {
        self.graph_register = f;
        self
    }

    pub async fn claim_and_run_one(&self) -> Result<bool, anyhow::Error> {
        let node_id = self.node_id;

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
        // tenant_id was stamped into the task payload by the scheduler. Older
        // pre-tenancy enqueues didn't carry one — fall back to DEFAULT_TENANT
        // (the all-zero UUID used by self-hosted deployments).
        let tenant: TenantId = task
            .payload
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_TENANT);

        let conn = match catalog_sql::load_connector(self.catalog.pool(), tenant, connector_id)
            .await?
        {
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

        let cursor = catalog_sql::load_cursor(self.catalog.pool(), tenant, connector_id).await?;
        let metrics = ConnectorMetrics {
            connector_id,
            type_id: impl_arc.type_id(),
        };
        let ctx = ConnectorCtx {
            connector_id,
            tenant,
            http: reqwest::Client::builder().build()?,
            secrets: self.secrets.clone(),
            credentials: self.credentials.clone(),
            oauth: self.oauth.clone(),
            scheduled_for,
            metrics: metrics.clone(),
        };

        let t0 = std::time::Instant::now();
        let outcome = impl_arc
            .run_once(&ctx, &conn.config_jsonb, cursor.as_ref())
            .await;

        match outcome {
            Ok(ConnectorRun {
                rows,
                new_cursor,
                tables,
                graph,
            }) => {
                // ---- Legacy single-table path (run.rows) ----
                let legacy_rows = rows.len() as u64;
                if !rows.is_empty() {
                    let idem = format!(
                        "connector:{}:{}",
                        connector_id,
                        scheduled_for_ms * 1_000_000
                    );
                    // Sink → WritePath. A failure here (table missing, schema
                    // mismatch, object-store down, CAS exhaustion) is treated as
                    // Transient so background_tasks retries within the attempt
                    // budget instead of orphaning the claimed task.
                    if let Err(e) = (self.sink)(
                        conn.target_database.clone(),
                        conn.target_table.clone(),
                        rows,
                        Some(idem),
                    )
                    .await
                    {
                        let msg = format!("sink: {e}");
                        warn!(connector_id = %connector_id, error = %msg, "sink failed");
                        catalog_sql::mark_run_failure(
                            self.catalog.pool(),
                            tenant,
                            connector_id,
                            &msg,
                        )
                        .await?;
                        self.catalog.fail_task(task.id, &msg).await?;
                        metrics.record_error("sink");
                        metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                        return Ok(true);
                    }
                }

                // ---- Multi-table path (run.tables) ----
                // Each table gets its own idempotency key so tables within the
                // same tick don't deduplicate each other (the critical
                // per-table-key invariant).
                let mut multi_rows: u64 = 0;
                for table_rows in tables {
                    let table_name = table_rows.table;
                    let idem = format!(
                        "connector:{}:{}:{}",
                        connector_id,
                        table_name,
                        scheduled_for_ms * 1_000_000
                    );
                    let n = table_rows.rows.len() as u64;
                    if let Err(e) = (self.sink)(
                        conn.target_database.clone(),
                        table_name.clone(),
                        table_rows.rows,
                        Some(idem),
                    )
                    .await
                    {
                        let msg = format!("sink({table_name}): {e}");
                        warn!(connector_id = %connector_id, table = %table_name, error = %msg, "multi-table sink failed");
                        catalog_sql::mark_run_failure(
                            self.catalog.pool(),
                            tenant,
                            connector_id,
                            &msg,
                        )
                        .await?;
                        self.catalog.fail_task(task.id, &msg).await?;
                        metrics.record_error("sink");
                        metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                        return Ok(true);
                    }
                    multi_rows += n;
                }

                // ---- Graph auto-registration ----
                if let Some(hint) = graph {
                    if let Err(e) =
                        (self.graph_register)(conn.target_database.clone(), hint).await
                    {
                        let msg = format!("graph_register: {e}");
                        warn!(connector_id = %connector_id, error = %msg, "graph registration failed");
                        catalog_sql::mark_run_failure(
                            self.catalog.pool(),
                            tenant,
                            connector_id,
                            &msg,
                        )
                        .await?;
                        self.catalog.fail_task(task.id, &msg).await?;
                        metrics.record_error("graph_register");
                        metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                        return Ok(true);
                    }
                }

                let n_rows = legacy_rows + multi_rows;
                if let Some(c) = new_cursor {
                    if let Err(e) =
                        catalog_sql::upsert_cursor(self.catalog.pool(), tenant, connector_id, &c)
                            .await
                    {
                        let msg = format!("cursor upsert: {e}");
                        warn!(connector_id = %connector_id, error = %msg, "cursor upsert failed");
                        catalog_sql::mark_run_failure(
                            self.catalog.pool(),
                            tenant,
                            connector_id,
                            &msg,
                        )
                        .await?;
                        self.catalog.fail_task(task.id, &msg).await?;
                        metrics.record_error("cursor");
                        metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                        return Ok(true);
                    }
                }
                if let Err(e) = catalog_sql::mark_run_success(
                    self.catalog.pool(),
                    tenant,
                    connector_id,
                    n_rows as i64,
                )
                .await
                {
                    error!(connector_id = %connector_id, error = %e, "mark_run_success failed");
                    // Fall through — still attempt to complete the task so it
                    // doesn't orphan. The run itself succeeded; only the status
                    // update failed.
                }
                self.catalog.complete_task(task.id).await?;
                metrics.record_rows(n_rows);
                metrics.record_tick(TickResult::Ok, t0.elapsed().as_secs_f64());
                metrics.set_last_success(Utc::now());
                info!(connector_id = %connector_id, rows = n_rows, "tick ok");
                Ok(true)
            }
            Err(ConnectorError::Transient(msg)) => {
                warn!(connector_id = %connector_id, error = %msg, "transient");
                catalog_sql::mark_run_failure(self.catalog.pool(), tenant, connector_id, &msg)
                    .await?;
                self.catalog.fail_task(task.id, &msg).await?;
                metrics.record_error("transient");
                metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                Ok(true)
            }
            Err(ConnectorError::Permanent(msg)) => {
                warn!(connector_id = %connector_id, error = %msg, "permanent");
                catalog_sql::mark_run_failure(self.catalog.pool(), tenant, connector_id, &msg)
                    .await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_error("permanent");
                metrics.record_tick(TickResult::Permanent, t0.elapsed().as_secs_f64());
                Ok(true)
            }
            Err(ConnectorError::Config(msg)) => {
                error!(connector_id = %connector_id, error = %msg, "config");
                catalog_sql::disable_connector(self.catalog.pool(), tenant, connector_id, &msg)
                    .await?;
                self.catalog.complete_task(task.id).await?;
                metrics.record_error("config");
                metrics.record_tick(TickResult::Config, t0.elapsed().as_secs_f64());
                Ok(true)
            }
        }
    }

    /// Recover abandoned claims from previous engine processes — any
    /// connector_tick row left in `claimed` whose `claim_expires_at` is in the
    /// past is marked `failed` so the queue self-heals across restarts.
    /// Without this, a sigkill or panic mid-tick could permanently jam the
    /// queue. Idempotent; safe to call from every worker on startup.
    pub async fn recover_stale_claims(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE background_tasks
             SET status = 'failed',
                 last_error = COALESCE(last_error, '') || ' [recovered: stale claim on restart]',
                 updated_at = now()
             WHERE kind = 'connector_tick'
               AND status = 'claimed'
               AND claim_expires_at < now()",
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(node_id = %self.node_id, "connector runner starting");
        // Best-effort one-shot recovery on startup. If multiple workers race
        // this in parallel, the SQL `UPDATE … WHERE status = 'claimed'` is
        // idempotent — the second pass simply finds nothing to update.
        match Self::recover_stale_claims(self.catalog.pool()).await {
            Ok(0) => {}
            Ok(n) => info!(stale_claims_recovered = n, "queue self-heal"),
            Err(e) => warn!(error = %e, "stale-claim recovery failed (non-fatal)"),
        }
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

/// Default [`CredentialStore`] used until `with_credentials` is called. Returns
/// `not found` on every lookup so connectors that genuinely need a credential
/// fail with a clear error instead of silently using uninitialized state. Also
/// handy in tests that build a [`ConnectorCtx`] for credential-free connectors.
pub struct NoopCredentialStore;
#[async_trait::async_trait]
impl CredentialStore for NoopCredentialStore {
    async fn get(
        &self,
        _tenant: TenantId,
        id: Uuid,
    ) -> anyhow::Result<kyma_core::credentials::Credential> {
        Err(anyhow::anyhow!(
            "credential store not configured (looked up id={id})"
        ))
    }
}
