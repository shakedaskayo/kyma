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

/// Default visibility policy (S3.3) for newly-written memories in a tenant.
/// `Public` (default) keeps the pre-feature behavior — every memory is shared.
/// `Private` defaults new memories to `private:<writer-subject>`, so agents see
/// only public memories plus their own unless a memory is explicitly shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemoryVisibility {
    #[default]
    Public,
    Private,
}

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

    // ── dreaming ────────────────────────────────────────────────────────────
    /// Scheduled agentic memory housekeeping (OFF by default).
    pub dreaming: DreamingSettings,

    // ── human-in-the-loop ─────────────────────────────────────────────────────
    /// Approval policy over automatic memory mutations (OFF by default — when
    /// disabled the system behaves exactly as before this feature shipped).
    pub hitl: super::memory_policy::HitlPolicy,

    // ── memory spaces (S3.3) ──────────────────────────────────────────────────
    /// Default visibility for new memories. `Public` (default) preserves the
    /// pre-feature shared-memory behavior.
    pub default_visibility: MemoryVisibility,
}

impl MemorySettings {
    /// Resolve the `space` value to store for a memory given an `explicit` space
    /// (writer-supplied, wins when present), the `writer_subject` (the
    /// authenticated principal's identity), and this tenant's default policy.
    ///
    /// Returns `None` (shared/public — no `space` written) unless the tenant
    /// defaults to `Private` and a subject is known, in which case new memories
    /// are scoped to `private:<subject>`. A subject-less write under a private
    /// default stays public (we never mint an un-attributable private space).
    pub fn resolve_space(
        &self,
        explicit: Option<&str>,
        writer_subject: Option<&str>,
    ) -> Option<String> {
        if let Some(s) = explicit {
            return Some(s.to_string());
        }
        match (self.default_visibility, writer_subject) {
            (MemoryVisibility::Private, Some(subj)) if !subj.is_empty() => {
                Some(format!("private:{subj}"))
            }
            _ => None,
        }
    }
}

/// Knobs for the scheduled dreaming pipeline — an autonomous agent run that
/// housekeeps the memory store (importance, relationships, dedup, archival)
/// and fills gaps with read-only data source access. `#[serde(default)]` keeps
/// older settings rows loading.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DreamingSettings {
    /// Master switch — dreaming never runs unless explicitly enabled.
    pub enabled: bool,
    /// Seconds between scheduled runs (default daily).
    pub interval_secs: u64,
    /// `full` | `housekeeping_only` | `sources`.
    pub mode: String,
    /// Realms in scope; empty = all realms.
    pub realm_scope: Vec<String>,
    /// Agent-loop budget: max tool calls per run (adk engines).
    pub max_tool_calls: u32,
    /// Wall-clock budget per run, seconds (all engines). Agentic dreaming runs
    /// (tool loops + LLM calls + data source reads) routinely need far longer than
    /// a typical request, so this defaults generously (1h); the `max_tool_calls`
    /// / `mutation_cap` / data source budgets are the real per-run guardrails.
    pub wall_clock_secs: u64,
    /// Gap-fill budget: max data_source_read calls per run.
    ///
    /// `alias`: settings persisted before the connectors → data-sources
    /// rename (027) carry the old key; the alias keeps those loading instead
    /// of silently resetting the operator's budget to the default. Aliases
    /// affect deserialization only — we always write the new key.
    #[serde(alias = "connector_read_budget")]
    pub data_source_read_budget: u32,
    /// Gap-fill budget: max bytes fetched across all data source reads.
    #[serde(alias = "connector_read_max_bytes")]
    pub data_source_read_max_bytes: u64,
    /// Cap on memory mutations (save/merge/archive/judge/…) per run.
    pub mutation_cap: u32,
}

impl Default for DreamingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 86_400,
            mode: "full".into(),
            realm_scope: vec![],
            max_tool_calls: 100,
            wall_clock_secs: 3_600,
            data_source_read_budget: 25,
            data_source_read_max_bytes: 4 * 1024 * 1024,
            mutation_cap: 60,
        }
    }
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
            dreaming: DreamingSettings::default(),
            hitl: super::memory_policy::HitlPolicy::default(),
            default_visibility: MemoryVisibility::Public,
        }
    }
}

/// Load the tenant's settings, falling back to defaults when unset or on any
/// read/parse error (never fails — recall/ingestion must keep working). In
/// **local mode** there is no Postgres pool (`None`) — settings are the
/// defaults, which is exactly the desired behavior.
pub async fn load(pool: Option<&PgPool>, tenant: TenantId) -> MemorySettings {
    let Some(pool) = pool else {
        return MemorySettings::default();
    };
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

/// Load settings the right way for this state: from the Postgres row in server
/// mode, or from the local JSON file (`memory_settings_path`) in local mode.
/// Never fails — falls back to defaults.
pub async fn load_for(state: &super::state::AgentState) -> MemorySettings {
    if let Some(pool) = state.pool.as_ref() {
        return load(Some(pool), state.tenant).await;
    }
    if let Some(path) = state.memory_settings_path.as_ref() {
        return load_local(path).await;
    }
    MemorySettings::default()
}

/// Load settings from a local JSON file. Missing/unparseable ⇒ defaults.
pub async fn load_local(path: &std::path::Path) -> MemorySettings {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => MemorySettings::default(),
    }
}

/// Persist settings to a local JSON file (local mode's stand-in for the
/// Postgres `memory_settings` row).
pub async fn save_local(path: &std::path::Path, s: &MemorySettings) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    tokio::fs::write(path, serde_json::to_string_pretty(s)?).await?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_settings_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("memory-settings.json");
        // Missing file ⇒ defaults.
        let d = load_local(&path).await;
        assert!(!d.dreaming.enabled);

        // Round-trip a customized dreaming block.
        let mut s = MemorySettings::default();
        s.dreaming.enabled = true;
        s.dreaming.wall_clock_secs = 180;
        s.dreaming.mutation_cap = 6;
        save_local(&path, &s).await.unwrap();

        let loaded = load_local(&path).await;
        assert!(loaded.dreaming.enabled);
        assert_eq!(loaded.dreaming.wall_clock_secs, 180);
        assert_eq!(loaded.dreaming.mutation_cap, 6);
    }

    #[test]
    fn legacy_settings_row_without_hitl_loads_default_off() {
        // A settings row written before the HITL field existed must still load,
        // with the policy defaulting to off (zero behavior change on upgrade).
        let legacy = serde_json::json!({
            "extraction_enabled": true,
            "min_events": 1,
            "dreaming": { "enabled": true }
        });
        let s: MemorySettings = serde_json::from_value(legacy).unwrap();
        assert!(!s.hitl.enabled, "HITL must default off for legacy rows");
        assert!(s.dreaming.enabled, "existing fields still load");
        assert_eq!(
            s.default_visibility,
            MemoryVisibility::Public,
            "legacy rows default to public visibility (no behavior change)"
        );
    }

    #[test]
    fn resolve_space_applies_tenant_default() {
        // Public default → no space (shared), regardless of subject.
        let public = MemorySettings::default();
        assert_eq!(public.resolve_space(None, Some("agentA")), None);

        // Private default + known subject → scoped to that subject.
        let mut private = MemorySettings::default();
        private.default_visibility = MemoryVisibility::Private;
        assert_eq!(
            private.resolve_space(None, Some("agentA")),
            Some("private:agentA".to_string())
        );
        // Private default but no subject → stays public (never mint an
        // un-attributable private space).
        assert_eq!(private.resolve_space(None, None), None);
        assert_eq!(private.resolve_space(None, Some("")), None);
        // An explicit space always wins, under either policy.
        assert_eq!(
            private.resolve_space(Some("public"), Some("agentA")),
            Some("public".to_string())
        );
        assert_eq!(
            public.resolve_space(Some("private:other"), None),
            Some("private:other".to_string())
        );
    }

    #[test]
    fn visibility_serde_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&MemoryVisibility::Private).unwrap(),
            "\"private\""
        );
        let v: MemoryVisibility = serde_json::from_str("\"public\"").unwrap();
        assert_eq!(v, MemoryVisibility::Public);
    }

    #[test]
    fn pre_rename_dreaming_budget_keys_still_load() {
        // Settings persisted before the connectors → data-sources rename
        // (e.g. kyma-local's JSON file, which no SQL migration touches) carry
        // the old field names — the serde aliases must map them instead of
        // silently resetting the budgets to defaults.
        let legacy = serde_json::json!({
            "dreaming": {
                "enabled": true,
                "connector_read_budget": 7,
                "connector_read_max_bytes": 123_456
            }
        });
        let s: MemorySettings = serde_json::from_value(legacy).unwrap();
        assert_eq!(s.dreaming.data_source_read_budget, 7);
        assert_eq!(s.dreaming.data_source_read_max_bytes, 123_456);
        // Wire output stays new-keyed.
        let v = serde_json::to_value(&s).unwrap();
        assert!(v["dreaming"].get("connector_read_budget").is_none());
        assert_eq!(v["dreaming"]["data_source_read_budget"], 7);
    }

    #[test]
    fn hitl_roundtrips_through_settings_json() {
        let mut s = MemorySettings::default();
        s.hitl.enabled = true;
        s.hitl.confidence_threshold = 0.8;
        let v = serde_json::to_value(&s).unwrap();
        let back: MemorySettings = serde_json::from_value(v).unwrap();
        assert!(back.hitl.enabled);
        assert_eq!(back.hitl.confidence_threshold, 0.8);
    }
}
