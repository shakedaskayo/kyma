//! User-tunable memory settings — the knobs surfaced in the Memory → Settings
//! UI so operators can tune ingestion and agentic-memory recall without
//! redeploying. Persisted as a single JSONB row per tenant in `memory_settings`
//! and read by the consolidation pipeline ([`super::memory`]) and the recall
//! orchestrator ([`super::memory_retrieve`]).
//!
//! `#[serde(default)]` on the struct means a row written by an older build
//! (missing newer fields) still loads — absent fields fall back to the coded
//! defaults rather than failing the whole load.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use kyma_core::tenant::TenantId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySettings {
    // ── ingestion ───────────────────────────────────────────────────────────
    /// Run LLM extraction + conflict resolution (vs deterministic summaries).
    pub extraction_enabled: bool,
    /// Minimum new firehose events in a realm before it is consolidated.
    pub min_events: i64,

    // ── retrieval / ranking ──────────────────────────────────────────────────
    /// Default recall limit when a query doesn't specify one.
    pub default_limit: usize,
    /// Default graph-expansion hops (0–2) when a query doesn't specify.
    pub default_expand_hops: u8,
    /// Native-ANN cosine-distance threshold; `0` disables ANN pruning
    /// (exact full-scan recall).
    pub ann_threshold: f64,
    /// Hybrid-blend weights (see `memory_retrieve::finalize`).
    pub w_rrf: f64,
    pub w_semantic: f64,
    pub w_keyword: f64,
    pub w_graph: f64,
    pub w_importance: f64,
    pub w_recency: f64,
    /// Recency half-life in days (`exp(-ln2·age/half_life)`).
    pub half_life_days: f64,
    /// Reciprocal-rank-fusion constant `1/(rrf_k + rank)`.
    pub rrf_k: f64,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            extraction_enabled: true,
            min_events: 1,
            default_limit: 8,
            default_expand_hops: 1,
            ann_threshold: 0.0,
            w_rrf: kyma_memory::W_RRF,
            w_semantic: kyma_memory::W_SEMANTIC,
            w_keyword: kyma_memory::W_KEYWORD,
            w_graph: kyma_memory::W_GRAPH,
            w_importance: kyma_memory::W_IMPORTANCE,
            w_recency: kyma_memory::W_RECENCY,
            half_life_days: kyma_memory::HALF_LIFE_DAYS,
            rrf_k: kyma_memory::RRF_K,
        }
    }
}

/// Load the tenant's settings, falling back to defaults when unset or on any
/// read/parse error (never fails — recall/ingestion must keep working).
pub async fn load(pool: &PgPool, tenant: TenantId) -> MemorySettings {
    let row: Option<(serde_json::Value,)> =
        sqlx::query_as("SELECT settings FROM memory_settings WHERE tenant_id = $1")
            .bind(tenant.as_uuid())
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match row {
        Some((v,)) => serde_json::from_value(v).unwrap_or_default(),
        None => MemorySettings::default(),
    }
}

/// Upsert the tenant's settings.
pub async fn save(pool: &PgPool, tenant: TenantId, s: &MemorySettings) -> anyhow::Result<()> {
    let json = serde_json::to_value(s)?;
    sqlx::query(
        "INSERT INTO memory_settings (tenant_id, settings, updated_at) \
         VALUES ($1, $2, now()) \
         ON CONFLICT (tenant_id) DO UPDATE SET settings = $2, updated_at = now()",
    )
    .bind(tenant.as_uuid())
    .bind(json)
    .execute(pool)
    .await?;
    Ok(())
}
