//! Shared handler state for the `/v1/agent/*` surface.

use crate::agent::engine::EnginePreferenceStore;
use crate::agent::skills::EnabledSkillsStore;
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
    /// Postgres pool — used to persist `agent_runs`/session rows and read memory
    /// settings. `None` in **local mode** (`kyma-local serve`): run/session
    /// history isn't persisted and settings default; memory recall/save and the
    /// data tools run over the catalog + engine and work unchanged.
    pub pool: Option<PgPool>,
    /// Persisted engine preference (provider/model/credential/host/extras).
    pub engines: Arc<dyn EnginePreferenceStore>,
    /// Tenant-scoped credential store — used by CredentialResolver.
    pub credentials: Arc<dyn CredentialStore>,
    /// Tenant id this server is scoped to. v1 single-tenant: `DEFAULT_TENANT`.
    pub tenant: TenantId,
    /// Enabled-skill names — drives system-prompt injection for non-CLI engines.
    pub skills: Arc<dyn EnabledSkillsStore>,
    /// Loopback URL of this server's own MCP endpoint (e.g.
    /// `http://127.0.0.1:8080/mcp/v1`). When set, the Claude CLI engine wires
    /// it via `--mcp-config` so the agent can query the user's data. `None`
    /// disables MCP wiring for that engine.
    pub mcp_url: Option<String>,
    /// Async memory ingest queue — flows into every [`super::SharedToolCtx`]
    /// built from this state. `None` ⇒ synchronous memory writes.
    pub memory: Option<kyma_memory::MemoryQueue>,
}
