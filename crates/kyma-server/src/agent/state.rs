//! Shared handler state for the `/v1/agent/*` surface.

use crate::agent::engine::EnginePreferenceStore;
use kyma_core::catalog::Catalog;
use kyma_core::credentials::CredentialStore;
use kyma_core::segment_format::SegmentFormat;
use kyma_core::tenant::TenantId;
use sqlx::PgPool;
use std::sync::Arc;

#[derive(Clone)]
pub struct AgentState {
    /// Catalog handle — used by tools to enumerate databases / tables.
    pub catalog: Arc<dyn Catalog>,
    /// Object-store + segment format — passed to KymaTable for inline tool SQL execution.
    pub format: Arc<dyn SegmentFormat>,
    /// Postgres pool — used to persist `agent_runs` rows on completion.
    pub pool: PgPool,
    /// Persisted engine preference (provider/model/credential/host/extras).
    pub engines: Arc<dyn EnginePreferenceStore>,
    /// Tenant-scoped credential store — used by CredentialResolver.
    pub credentials: Arc<dyn CredentialStore>,
    /// Tenant id this server is scoped to. v1 single-tenant: `DEFAULT_TENANT`.
    pub tenant: TenantId,
}
