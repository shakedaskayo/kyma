//! kyma-federation — federated (live-proxied) external data sources.
//!
//! External data platforms (Microsoft Fabric, and later Databricks, Snowflake,
//! BigQuery) hold data kyma should never ingest wholesale. Instead, a
//! *federated data source* (e.g. `msfabric` in `kyma-datasources`) syncs only
//! table **schemas** into the catalog — marking each table with
//! [`FederatedTableSpec`] in its `TableConfig` — and this crate turns those
//! metadata-only tables into live remote scans at query time.
//!
//! Mechanism: each federated table is registered in the DataFusion
//! `SessionContext` as a [`datafusion_federation::FederatedTableProviderAdaptor`]
//! instead of a `KymaTable`. The `FederationOptimizerRule` then carves out the
//! largest subplans whose tables share a remote source, unparses them back to
//! SQL in the platform's dialect, and ships the whole subplan (filters, joins
//! between two remote tables, aggregates) to the platform for execution.
//! Mixed local⋈remote plans degrade gracefully: the remote side becomes one
//! remote query, the join runs locally.
//!
//! Tables on the **same** source — identical `(platform, endpoint,
//! remote_database, credential_id)` — must share one `SQLFederationProvider`
//! instance per query, or the optimizer treats them as different engines and
//! refuses to push joins down. [`FederationRuntime::federated_providers`]
//! guarantees that grouping.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use datafusion::catalog::TableProvider;
use datafusion::execution::context::SessionContext;
use datafusion::execution::runtime_env::RuntimeEnv;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::prelude::SessionConfig;
use datafusion_federation::sql::{SQLFederationProvider, SQLTableSource};
use datafusion_federation::{
    FederatedQueryPlanner, FederatedTableProviderAdaptor, FederatedTableSource,
    FederationOptimizerRule,
};
use kyma_core::catalog::{FederatedTableSpec, TableRef};
use kyma_core::credentials::{CredentialStore, CredentialValue};
use kyma_core::tenant::TenantId;

pub mod dialect;
pub mod msfabric;

/// Platform id handled by [`msfabric`]. Other platforms (databricks, snowflake,
/// bigquery) are follow-up implementations of the same shape.
pub use msfabric::PLATFORM_ID as MSFABRIC;

/// Server-level limits applied to every remote query. Live queries run on the
/// external platform's compute (someone else's bill) — keep them bounded.
#[derive(Debug, Clone)]
pub struct FederationGuardrails {
    /// Wall-clock budget for one remote query (connect + execute + fetch).
    pub remote_timeout: Duration,
    /// Max rows fetched per remote query; excess rows are dropped with a
    /// warning (the result is marked truncated in logs).
    pub max_rows: usize,
    /// Max concurrent in-flight remote queries per source.
    pub max_concurrent_per_source: usize,
}

impl Default for FederationGuardrails {
    fn default() -> Self {
        Self {
            remote_timeout: Duration::from_secs(30),
            max_rows: 100_000,
            max_concurrent_per_source: 4,
        }
    }
}

impl FederationGuardrails {
    /// Defaults overridable via `KYMA_FEDERATION_TIMEOUT_MS`,
    /// `KYMA_FEDERATION_MAX_ROWS`, `KYMA_FEDERATION_MAX_CONCURRENT`.
    pub fn from_env() -> Self {
        let env_u64 = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
        let d = Self::default();
        Self {
            remote_timeout: env_u64("KYMA_FEDERATION_TIMEOUT_MS")
                .map_or(d.remote_timeout, Duration::from_millis),
            max_rows: env_u64("KYMA_FEDERATION_MAX_ROWS").map_or(d.max_rows, |v| v as usize),
            max_concurrent_per_source: env_u64("KYMA_FEDERATION_MAX_CONCURRENT")
                .map_or(d.max_concurrent_per_source, |v| v as usize),
        }
    }
}

/// Convenience: a runtime over `credentials` with env-tuned guardrails. Cheap
/// to call per request — token caches are process-wide, not per-runtime.
pub fn runtime_from(credentials: Arc<dyn CredentialStore>) -> Arc<FederationRuntime> {
    Arc::new(FederationRuntime::new(
        credentials,
        FederationGuardrails::from_env(),
    ))
}

/// One remote source = one remote compute context. Tables sharing a key share
/// a `SQLFederationProvider`, which is what lets same-source joins federate
/// into a single remote SQL statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SourceKey {
    platform: String,
    endpoint: String,
    remote_database: String,
    credential_id: uuid::Uuid,
}

impl SourceKey {
    fn of(spec: &FederatedTableSpec) -> Self {
        Self {
            platform: spec.platform.clone(),
            endpoint: spec.endpoint.clone(),
            remote_database: spec.remote_database.clone(),
            credential_id: spec.credential_id,
        }
    }
}

/// Long-lived handle that builds live `TableProvider`s for federated tables.
/// Owns the credential store and guardrails; cheap to clone (everything is
/// behind `Arc`s). Token caches live process-wide inside the platform modules,
/// so per-query provider construction stays cheap.
#[derive(Clone)]
pub struct FederationRuntime {
    credentials: Arc<dyn CredentialStore>,
    guardrails: FederationGuardrails,
}

impl FederationRuntime {
    pub fn new(credentials: Arc<dyn CredentialStore>, guardrails: FederationGuardrails) -> Self {
        Self {
            credentials,
            guardrails,
        }
    }

    pub fn guardrails(&self) -> &FederationGuardrails {
        &self.guardrails
    }

    /// Build a live `TableProvider` per federated table in `tables`
    /// (non-federated tables are ignored). Returned pairs are
    /// `(kyma table name, provider)`, ready for
    /// `SessionContext::register_table`.
    ///
    /// Tables on the same source share one `SQLFederationProvider`, so joins
    /// between them federate into a single remote query.
    pub async fn federated_providers(
        &self,
        tenant: TenantId,
        tables: &[TableRef],
    ) -> anyhow::Result<Vec<(String, Arc<dyn TableProvider>)>> {
        let mut providers: HashMap<SourceKey, Arc<SQLFederationProvider>> = HashMap::new();
        let mut out: Vec<(String, Arc<dyn TableProvider>)> = Vec::new();

        for table in tables {
            let Some(spec) = table.config.federated.as_ref() else {
                continue;
            };
            let key = SourceKey::of(spec);
            let provider = match providers.get(&key) {
                Some(p) => p.clone(),
                None => {
                    let p = self.provider_for(tenant, spec).await?;
                    providers.insert(key, p.clone());
                    p
                }
            };
            let remote_name = format!("{}.{}", spec.remote_schema, spec.remote_table);
            let source = SQLTableSource::new_with_schema(
                provider,
                remote_name,
                table.schema.clone(),
            )
            .map_err(|e| anyhow::anyhow!("federated table source {}: {e}", table.name))?;
            let source: Arc<dyn FederatedTableSource> = Arc::new(source);
            out.push((
                table.name.clone(),
                Arc::new(FederatedTableProviderAdaptor::new(source)),
            ));
        }
        Ok(out)
    }

    /// Resolve the credential and build one federation provider for a source.
    async fn provider_for(
        &self,
        tenant: TenantId,
        spec: &FederatedTableSpec,
    ) -> anyhow::Result<Arc<SQLFederationProvider>> {
        let credential = self
            .credentials
            .get(tenant, spec.credential_id)
            .await
            .map_err(|e| anyhow::anyhow!("credential {} for federated source: {e}", spec.credential_id))?;
        match spec.platform.as_str() {
            msfabric::PLATFORM_ID => {
                let CredentialValue::ServicePrincipal { .. } = &credential.value else {
                    anyhow::bail!(
                        "federated platform `{}` requires a service_principal credential, got `{}`",
                        spec.platform,
                        credential.value.kind()
                    );
                };
                let executor = msfabric::MsFabricExecutor::new(
                    &spec.endpoint,
                    &spec.remote_database,
                    &credential.value,
                    spec.credential_id,
                    &self.guardrails,
                )?;
                Ok(Arc::new(SQLFederationProvider::new(Arc::new(executor))))
            }
            other => anyhow::bail!("unknown federated platform `{other}`"),
        }
    }
}

/// True when any table in the slice is federated — used by query paths to
/// decide whether the (slightly heavier) federation-aware `SessionContext`
/// is needed.
pub fn any_federated(tables: &[TableRef]) -> bool {
    tables.iter().any(|t| t.config.federated.is_some())
}

/// Build a `SessionContext` with the federation optimizer rule + query planner
/// installed on top of DataFusion's defaults. Drop-in replacement for
/// `SessionContext::new_with_config_rt` at the query-path call sites: plans
/// without federated tables are untouched by the extra rule.
pub fn federated_session_context(
    config: SessionConfig,
    runtime: Arc<RuntimeEnv>,
) -> SessionContext {
    let state = SessionStateBuilder::new()
        .with_config(config)
        .with_runtime_env(runtime)
        .with_default_features()
        .with_optimizer_rule(Arc::new(FederationOptimizerRule::new()))
        .with_query_planner(Arc::new(FederatedQueryPlanner::new()))
        .build();
    SessionContext::new_with_state(state)
}
