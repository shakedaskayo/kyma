//! Shared handler state for the `/v1/agent/*` surface.

use kyma_core::catalog::Catalog;
use kyma_core::segment_format::SegmentFormat;
use sqlx::PgPool;
use std::sync::Arc;

/// Shared axum handler state for the agent endpoints.
///
/// Combines the same catalog + segment-format pair used by the main
/// `/v1/query` surface with a Postgres pool for `agent_runs` persistence.
#[derive(Clone)]
pub struct AgentState {
    /// Catalog handle — used by tools to enumerate databases / tables
    /// and to register [`KymaTable`](kyma_exec::KymaTable) instances for
    /// `run_sql` / `sample_rows`.
    pub catalog: Arc<dyn Catalog>,
    /// Object-store + segment format — passed to [`KymaTable`](kyma_exec::KymaTable)
    /// for inline tool SQL execution.
    pub format: Arc<dyn SegmentFormat>,
    /// Postgres pool — used to persist `agent_runs` rows on completion.
    pub pool: PgPool,
}
