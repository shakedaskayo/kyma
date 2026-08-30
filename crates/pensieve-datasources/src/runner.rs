//! Data source tick runner.

use crate::catalog_sql;
use crate::metrics::{DataSourceMetrics, TickResult};
use crate::registry::DataSourceRegistry;
use crate::secrets::SecretStore;
use pensieve_core::credentials::CredentialStore;
use crate::types::{DataSourceCtx, DataSourceError, DataSourceRun, GraphHint};
use chrono::Utc;
use futures::future::BoxFuture;
use pensieve_catalog::PostgresCatalog;
use pensieve_core::catalog::Catalog;
use pensieve_core::tenant::{TenantId, DEFAULT_TENANT};
use pensieve_core::types::NodeId;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Thin async "send these rows into WritePath" closure. Passed in so the
/// runner does not need a generic on WritePath; that plumbing lives in
/// pensieve-bin where the concrete types are assembled.
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

/// DataSource-table operations the tick body performs (row + cursor + status).
/// In-server execution implements this with direct SQL ([`PgDataSourceControl`]);
/// remote fabric workers implement it over the control-plane HTTP API.
#[async_trait::async_trait]
pub trait DataSourceControl: Send + Sync {
    async fn load_data_source(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> anyhow::Result<Option<catalog_sql::DataSourceRow>>;
    async fn load_cursor(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> anyhow::Result<Option<serde_json::Value>>;
    async fn upsert_cursor(
        &self,
        tenant: TenantId,
        id: Uuid,
        cursor: &serde_json::Value,
    ) -> anyhow::Result<()>;
    async fn mark_run_success(&self, tenant: TenantId, id: Uuid, rows: i64) -> anyhow::Result<()>;
    async fn mark_run_failure(&self, tenant: TenantId, id: Uuid, msg: &str) -> anyhow::Result<()>;
    async fn disable_data_source(&self, tenant: TenantId, id: Uuid, msg: &str) -> anyhow::Result<()>;
}

/// Direct-SQL [`DataSourceControl`] for in-server execution.
pub struct PgDataSourceControl {
    pool: sqlx::PgPool,
}

impl PgDataSourceControl {
    pub fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl DataSourceControl for PgDataSourceControl {
    async fn load_data_source(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> anyhow::Result<Option<catalog_sql::DataSourceRow>> {
        Ok(catalog_sql::load_data_source(&self.pool, tenant, id).await?)
    }
    async fn load_cursor(
        &self,
        tenant: TenantId,
        id: Uuid,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(catalog_sql::load_cursor(&self.pool, tenant, id).await?)
    }
    async fn upsert_cursor(
        &self,
        tenant: TenantId,
        id: Uuid,
        cursor: &serde_json::Value,
    ) -> anyhow::Result<()> {
        Ok(catalog_sql::upsert_cursor(&self.pool, tenant, id, cursor).await?)
    }
    async fn mark_run_success(&self, tenant: TenantId, id: Uuid, rows: i64) -> anyhow::Result<()> {
        Ok(catalog_sql::mark_run_success(&self.pool, tenant, id, rows).await?)
    }
    async fn mark_run_failure(&self, tenant: TenantId, id: Uuid, msg: &str) -> anyhow::Result<()> {
        Ok(catalog_sql::mark_run_failure(&self.pool, tenant, id, msg).await?)
    }
    async fn disable_data_source(&self, tenant: TenantId, id: Uuid, msg: &str) -> anyhow::Result<()> {
        Ok(catalog_sql::disable_data_source(&self.pool, tenant, id, msg).await?)
    }
}

/// Everything one data source tick needs, independent of which queue delivered
/// it (background_tasks via [`DataSourceRunner`], or a fabric `datasource_sync`
/// job via pensieve-jobs' executor).
#[derive(Clone)]
pub struct DataSourceTickDeps {
    pub control: Arc<dyn DataSourceControl>,
    pub registry: Arc<DataSourceRegistry>,
    pub sink: RowSink,
    pub graph_register: GraphRegisterFn,
    pub secrets: Arc<dyn SecretStore>,
    pub credentials: Arc<dyn CredentialStore>,
    pub oauth: Option<crate::oauth::OAuthRuntime>,
    /// Object-store artifact capability for data sources that persist full-file
    /// blobs (e.g. CI job logs). `None` skips blob capture.
    pub artifacts: Option<Arc<dyn crate::artifacts::ArtifactStore>>,
    /// Catalog access for metadata-sync (federated) data sources. `None` for
    /// runners that only execute row-producing data sources.
    pub catalog: Option<Arc<dyn Catalog>>,
}

/// Terminal state of one tick, mapped by the queue front-end onto its own
/// complete/fail semantics (background_tasks today, fabric jobs on the new
/// path).
pub enum TickOutcome {
    /// Run succeeded; rows ingested.
    Ok { rows: u64 },
    /// Nothing to do (data source disabled or row missing) — treat as done.
    Skipped(&'static str),
    /// Retryable failure — the queue should retry within its attempt budget.
    Transient(String),
    /// Non-retryable failure — recorded on the data source; do not retry.
    Permanent(String),
    /// Broken configuration — the data source was disabled; do not retry.
    Config(String),
}

/// Run a single data source tick: load row + cursor, execute the data source,
/// sink rows (per-table idempotency keys), register graphs, persist the
/// cursor, and record run status on the data source row. Queue bookkeeping
/// (claim/complete/fail) is the caller's job.
pub async fn run_data_source_tick(
    deps: &DataSourceTickDeps,
    tenant: TenantId,
    data_source_id: Uuid,
    scheduled_for_ms: i64,
) -> anyhow::Result<TickOutcome> {
    let scheduled_for = chrono::DateTime::<Utc>::from_timestamp_millis(scheduled_for_ms)
        .ok_or_else(|| anyhow::anyhow!("bad scheduled_for"))?;

    let conn = match deps.control.load_data_source(tenant, data_source_id).await? {
        Some(c) if c.enabled => c,
        Some(_) => {
            debug!(data_source_id = %data_source_id, "skipping disabled data source");
            return Ok(TickOutcome::Skipped("disabled"));
        }
        None => {
            warn!(data_source_id = %data_source_id, "data source row missing");
            return Ok(TickOutcome::Skipped("missing"));
        }
    };

    let impl_arc = deps
        .registry
        .lookup(&conn.type_id)
        .ok_or_else(|| anyhow::anyhow!("no registered impl for type {}", conn.type_id))?;

    let cursor = deps.control.load_cursor(tenant, data_source_id).await?;
    let metrics = DataSourceMetrics {
        data_source_id,
        type_id: impl_arc.type_id(),
    };
    let ctx = DataSourceCtx {
        data_source_id,
        tenant,
        http: reqwest::Client::builder().build()?,
        secrets: deps.secrets.clone(),
        credentials: deps.credentials.clone(),
        oauth: deps.oauth.clone(),
        scheduled_for,
        metrics: metrics.clone(),
        artifacts: deps.artifacts.clone(),
        catalog: deps.catalog.clone(),
        target_database: conn.target_database.clone(),
    };

    let t0 = std::time::Instant::now();
    let outcome = impl_arc
        .run_once(&ctx, &conn.config_jsonb, cursor.as_ref())
        .await;

    match outcome {
        Ok(DataSourceRun {
            rows,
            new_cursor,
            tables,
            graph,
        }) => {
            // ---- Legacy single-table path (run.rows) ----
            let legacy_rows = rows.len() as u64;
            if !rows.is_empty() {
                let idem = format!(
                    "data_source:{}:{}",
                    data_source_id,
                    scheduled_for_ms * 1_000_000
                );
                // Sink → WritePath. A failure here (table missing, schema
                // mismatch, object-store down, CAS exhaustion) is Transient so
                // the queue retries within its attempt budget.
                if let Err(e) = (deps.sink)(
                    conn.target_database.clone(),
                    conn.target_table.clone(),
                    rows,
                    Some(idem),
                )
                .await
                {
                    let msg = format!("sink: {e}");
                    warn!(data_source_id = %data_source_id, error = %msg, "sink failed");
                    deps.control
                        .mark_run_failure(tenant, data_source_id, &msg)
                        .await?;
                    metrics.record_error("sink");
                    metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                    return Ok(TickOutcome::Transient(msg));
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
                    "data_source:{}:{}:{}",
                    data_source_id,
                    table_name,
                    scheduled_for_ms * 1_000_000
                );
                let n = table_rows.rows.len() as u64;
                if let Err(e) = (deps.sink)(
                    conn.target_database.clone(),
                    table_name.clone(),
                    table_rows.rows,
                    Some(idem),
                )
                .await
                {
                    let msg = format!("sink({table_name}): {e}");
                    warn!(data_source_id = %data_source_id, table = %table_name, error = %msg, "multi-table sink failed");
                    deps.control
                        .mark_run_failure(tenant, data_source_id, &msg)
                        .await?;
                    metrics.record_error("sink");
                    metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                    return Ok(TickOutcome::Transient(msg));
                }
                multi_rows += n;
            }

            // ---- Graph auto-registration ----
            if let Some(hint) = graph {
                if let Err(e) = (deps.graph_register)(conn.target_database.clone(), hint).await {
                    let msg = format!("graph_register: {e}");
                    warn!(data_source_id = %data_source_id, error = %msg, "graph registration failed");
                    deps.control
                        .mark_run_failure(tenant, data_source_id, &msg)
                        .await?;
                    metrics.record_error("graph_register");
                    metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                    return Ok(TickOutcome::Transient(msg));
                }
            }

            let n_rows = legacy_rows + multi_rows;
            if let Some(c) = new_cursor {
                if let Err(e) = deps.control.upsert_cursor(tenant, data_source_id, &c).await {
                    let msg = format!("cursor upsert: {e}");
                    warn!(data_source_id = %data_source_id, error = %msg, "cursor upsert failed");
                    deps.control
                        .mark_run_failure(tenant, data_source_id, &msg)
                        .await?;
                    metrics.record_error("cursor");
                    metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
                    return Ok(TickOutcome::Transient(msg));
                }
            }
            if let Err(e) = deps
                .control
                .mark_run_success(tenant, data_source_id, n_rows as i64)
                .await
            {
                error!(data_source_id = %data_source_id, error = %e, "mark_run_success failed");
                // Fall through — the run itself succeeded; only the status
                // update failed.
            }
            metrics.record_rows(n_rows);
            metrics.record_tick(TickResult::Ok, t0.elapsed().as_secs_f64());
            metrics.set_last_success(Utc::now());
            info!(data_source_id = %data_source_id, rows = n_rows, "tick ok");
            Ok(TickOutcome::Ok { rows: n_rows })
        }
        Err(DataSourceError::Transient(msg)) => {
            warn!(data_source_id = %data_source_id, error = %msg, "transient");
            deps.control
                .mark_run_failure(tenant, data_source_id, &msg)
                .await?;
            metrics.record_error("transient");
            metrics.record_tick(TickResult::Transient, t0.elapsed().as_secs_f64());
            Ok(TickOutcome::Transient(msg))
        }
        Err(DataSourceError::Permanent(msg)) => {
            warn!(data_source_id = %data_source_id, error = %msg, "permanent");
            deps.control
                .mark_run_failure(tenant, data_source_id, &msg)
                .await?;
            metrics.record_error("permanent");
            metrics.record_tick(TickResult::Permanent, t0.elapsed().as_secs_f64());
            Ok(TickOutcome::Permanent(msg))
        }
        Err(DataSourceError::Config(msg)) => {
            error!(data_source_id = %data_source_id, error = %msg, "config");
            deps.control
                .disable_data_source(tenant, data_source_id, &msg)
                .await?;
            metrics.record_error("config");
            metrics.record_tick(TickResult::Config, t0.elapsed().as_secs_f64());
            Ok(TickOutcome::Config(msg))
        }
    }
}

#[derive(Clone)]
pub struct DataSourceRunner {
    catalog: Arc<PostgresCatalog>,
    registry: Arc<DataSourceRegistry>,
    sink: RowSink,
    graph_register: GraphRegisterFn,
    secrets: Arc<dyn SecretStore>,
    /// System-wide credential store, passed into every `DataSourceCtx`.
    /// A no-op stub is supplied by default so existing builds (and tests)
    /// don't need wiring; callers attach the real store via
    /// [`Self::with_credentials`].
    credentials: Arc<dyn CredentialStore>,
    /// Run-time OAuth handle (pool + crypto) for token refresh of OAuth
    /// data sources. Attached via [`Self::with_oauth`]; `None` leaves refresh on
    /// operator-env client creds only.
    oauth: Option<crate::oauth::OAuthRuntime>,
    /// Object-store artifact capability, passed into every `DataSourceCtx`.
    /// Attached via [`Self::with_artifacts`]; `None` (the default) means
    /// blob-emitting data sources skip capture.
    artifacts: Option<Arc<dyn crate::artifacts::ArtifactStore>>,
    /// NodeId of the already-registered node (supplied by the caller).
    node_id: NodeId,
    pub idle_sleep: Duration,
    pub claim_lease: chrono::Duration,
}

impl DataSourceRunner {
    pub fn new<S: SecretStore + 'static>(
        catalog: Arc<PostgresCatalog>,
        registry: Arc<DataSourceRegistry>,
        sink: RowSink,
        secrets: S,
        node_id: NodeId,
    ) -> Self {
        // Default graph_register is a no-op; callers can supply a real one
        // via `with_graph_register`.
        let graph_register: GraphRegisterFn =
            Arc::new(|_db, _hint| Box::pin(async move { Ok(()) }));
        // No-op credentials store by default — data sources that don't look up
        // credentials work unchanged; the real store is attached via
        // `with_credentials` from pensieve-bin.
        let credentials: Arc<dyn CredentialStore> = Arc::new(NoopCredentialStore);
        Self {
            catalog,
            registry,
            sink,
            graph_register,
            secrets: Arc::new(secrets),
            credentials,
            oauth: None,
            artifacts: None,
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
    /// data sources that take inline secrets, required for ones that resolve a
    /// `credential_id`.
    pub fn with_credentials(mut self, store: Arc<dyn CredentialStore>) -> Self {
        self.credentials = store;
        self
    }

    /// Attach the run-time OAuth handle so OAuth data sources can refresh + persist
    /// expired access tokens (resolving per-tenant bring-your-own client creds
    /// from `oauth_clients` in addition to operator env).
    pub fn with_oauth(
        mut self,
        pool: sqlx::PgPool,
        crypto: Arc<pensieve_core::crypto::Crypto>,
    ) -> Self {
        self.oauth = Some(crate::oauth::OAuthRuntime { pool, crypto });
        self
    }

    /// Replace the graph-registration closure. Returns `self` for chaining.
    pub fn with_graph_register(mut self, f: GraphRegisterFn) -> Self {
        self.graph_register = f;
        self
    }

    /// Attach the object-store artifact capability so data sources can persist
    /// full-file blobs (CI job logs, etc.). Without this call, `ctx.artifacts`
    /// is `None` and blob-emitting data sources skip capture.
    pub fn with_artifacts(mut self, artifacts: Arc<dyn crate::artifacts::ArtifactStore>) -> Self {
        self.artifacts = Some(artifacts);
        self
    }

    /// Bundle this runner's wiring into the queue-independent tick deps.
    pub fn tick_deps(&self) -> DataSourceTickDeps {
        DataSourceTickDeps {
            control: Arc::new(PgDataSourceControl::new(self.catalog.pool().clone())),
            registry: self.registry.clone(),
            sink: self.sink.clone(),
            graph_register: self.graph_register.clone(),
            secrets: self.secrets.clone(),
            credentials: self.credentials.clone(),
            oauth: self.oauth.clone(),
            artifacts: self.artifacts.clone(),
            catalog: Some(self.catalog.clone() as Arc<dyn Catalog>),
        }
    }

    pub async fn claim_and_run_one(&self) -> Result<bool, anyhow::Error> {
        let node_id = self.node_id;

        let Some(task) = self
            .catalog
            .claim_task("data_source_tick", node_id, self.claim_lease)
            .await?
        else {
            return Ok(false);
        };

        let data_source_id = task
            .payload
            .get("data_source_id")
            .and_then(|v| v.as_str())
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| anyhow::anyhow!("task missing data_source_id"))?;
        let scheduled_for_ms = task
            .payload
            .get("scheduled_for")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or_else(|| anyhow::anyhow!("task missing scheduled_for"))?;
        // tenant_id was stamped into the task payload by the scheduler. Older
        // pre-tenancy enqueues didn't carry one — fall back to DEFAULT_TENANT
        // (the all-zero UUID used by self-hosted deployments).
        let tenant: TenantId = task
            .payload
            .get("tenant_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_TENANT);

        let deps = self.tick_deps();
        match run_data_source_tick(&deps, tenant, data_source_id, scheduled_for_ms).await? {
            TickOutcome::Ok { .. } | TickOutcome::Skipped(_) => {
                self.catalog.complete_task(task.id).await?;
            }
            // Retryable: background_tasks re-queues within the attempt budget.
            TickOutcome::Transient(msg) => {
                self.catalog.fail_task(task.id, &msg).await?;
            }
            // Non-retryable outcomes recorded on the data source; the task is done.
            TickOutcome::Permanent(_) | TickOutcome::Config(_) => {
                self.catalog.complete_task(task.id).await?;
            }
        }
        Ok(true)
    }

    /// Recover abandoned claims from previous engine processes — any
    /// data_source_tick row left in `claimed` whose `claim_expires_at` is in the
    /// past is marked `failed` so the queue self-heals across restarts.
    /// Without this, a sigkill or panic mid-tick could permanently jam the
    /// queue. Idempotent; safe to call from every worker on startup.
    pub async fn recover_stale_claims(pool: &sqlx::PgPool) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "UPDATE background_tasks
             SET status = 'failed',
                 last_error = COALESCE(last_error, '') || ' [recovered: stale claim on restart]',
                 updated_at = now()
             WHERE kind = 'data_source_tick'
               AND status = 'claimed'
               AND claim_expires_at < now()",
        )
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        info!(node_id = %self.node_id, "data source runner starting");
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
                () = &mut shutdown => { info!("data source runner shutdown"); return; }
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
/// `not found` on every lookup so data sources that genuinely need a credential
/// fail with a clear error instead of silently using uninitialized state. Also
/// handy in tests that build a [`DataSourceCtx`] for credential-free data sources.
pub struct NoopCredentialStore;
#[async_trait::async_trait]
impl CredentialStore for NoopCredentialStore {
    async fn get(
        &self,
        _tenant: TenantId,
        id: Uuid,
    ) -> anyhow::Result<pensieve_core::credentials::Credential> {
        Err(anyhow::anyhow!(
            "credential store not configured (looked up id={id})"
        ))
    }
}
