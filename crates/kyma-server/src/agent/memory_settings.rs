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
    /// Usage-based reinforcement blend weight (see `memory_retrieve::finalize`
    /// and [`ReinforcementSettings`]). Defaults to 0 — disabled — so enabling
    /// it is an explicit operator decision; existing rankings never silently
    /// shift on upgrade.
    pub w_reinforcement: f64,

    // ── usage-based reinforcement (M8.1) ──────────────────────────────────────
    pub reinforcement: ReinforcementSettings,

    // ── worked-example precedent retrieval (M8.2) ─────────────────────────────
    pub precedent: PrecedentSettings,

    // ── MMR diversity re-ranking + validity gate (M8.3) ───────────────────────
    pub mmr: MmrSettings,
    pub validity_gate: ValidityGateSettings,

    // ── schema/procedure induction (M8.4) ─────────────────────────────────────
    /// Folded into the existing dreaming pipeline as an extra phase (no
    /// separate scheduler/job) — see `dreaming_skill`'s PHASE 4.
    pub schema_induction: SchemaInductionSettings,

    // ── dreaming ────────────────────────────────────────────────────────────
    /// Scheduled agentic memory housekeeping (OFF by default).
    pub dreaming: DreamingSettings,

    // ── human-in-the-loop ─────────────────────────────────────────────────────
    /// Approval policy over automatic memory mutations (OFF by default — when
    /// disabled the system behaves exactly as before this feature shipped).
    pub hitl: super::memory_policy::HitlPolicy,
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

/// Knobs for the usage-based reinforcement loop (M8.1) — hit/miss tracking +
/// forgetting-curve decay layered on top of the existing recall blend. See
/// [`kyma_memory::reinforcement`]. `#[serde(default)]` keeps older settings
/// rows loading with the feature off.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReinforcementSettings {
    /// Master switch. When `false`, `w_reinforcement` is ignored regardless
    /// of its configured value — defense in depth alongside the 0.0 default.
    pub enabled: bool,
    /// Reinforced-hit boost / missed-hit penalty on the effective half-life.
    pub hit_weight: f64,
    pub miss_penalty: f64,
    /// Half-life (days) for the usage-decay term (independent of the
    /// recency-decay `half_life_days` above).
    pub half_life_days: f64,
}

impl Default for ReinforcementSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            hit_weight: kyma_memory::REINFORCEMENT_HIT_WEIGHT,
            miss_penalty: kyma_memory::REINFORCEMENT_MISS_PENALTY,
            half_life_days: kyma_memory::HALF_LIFE_DAYS,
        }
    }
}

/// Knobs for worked-example ("precedent") retrieval (M8.2) — see
/// [`kyma_memory::activities`]. Purely additive to a recall response (a new
/// `precedent` field, never blended into ranking), so unlike
/// [`ReinforcementSettings`] this defaults *on*.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrecedentSettings {
    pub enabled: bool,
    /// Cosine-distance cutoff for "this looks like the same input we saw
    /// before" — much stricter than ordinary semantic recall.
    pub max_distance: f64,
    /// Max memories attached per precedent activity.
    pub memory_limit: usize,
}

impl Default for PrecedentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_distance: kyma_memory::PRECEDENT_MAX_DISTANCE,
            memory_limit: 20,
        }
    }
}

/// Knobs for MMR (Maximal Marginal Relevance) diversity re-ranking (M8.3a) —
/// see `memory_retrieve::mmr_rerank`. Off by default: it's a ranking change,
/// so — like [`ReinforcementSettings`] — enabling it is an explicit operator
/// decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MmrSettings {
    pub enabled: bool,
    /// Relevance/diversity trade-off in `[0, 1]`: 1.0 = pure relevance (no
    /// diversity effect), lower values favor spreading out near-duplicates.
    pub lambda: f64,
    /// How many top-blended candidates to consider for re-ranking, as a
    /// multiple of the requested `limit`. Bounds the extra embedding fetch —
    /// the default (non-MMR) recall path never pays for it.
    pub pool_multiplier: usize,
}

impl Default for MmrSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            lambda: 0.7,
            pool_multiplier: 3,
        }
    }
}

/// Knobs for the optional validity gate (M8.3b) — see
/// [`super::memory_validity_gate`]. Off by default: rejecting content is a
/// behavior change (fewer memories saved), so it's opt-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidityGateSettings {
    pub enabled: bool,
    /// Escalate borderline (heuristic-flagged) content to a second-opinion
    /// LLM judge rather than trusting the heuristic alone. Costs one extra
    /// LLM turn per flagged candidate.
    pub llm_escalation_enabled: bool,
    /// Extraction-confidence floor (realtime pipeline only — reuses the
    /// extractor's own score, no extra LLM call).
    pub min_confidence: f32,
    /// Minimum content length (chars) below which content is rejected as
    /// too trivial to be a durable memory.
    pub min_content_chars: usize,
}

impl Default for ValidityGateSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            llm_escalation_enabled: false,
            min_confidence: 0.35,
            min_content_chars: 12,
        }
    }
}

/// Knobs for schema/procedure induction (M8.4) — see [`super::dreaming_skill`]
/// PHASE 4. Off by default. Deliberately no separate scheduling cadence: the
/// dreaming trigger prompt embeds `interval_days`/`min_examples` as numbers
/// and the skill itself checks (via `list_memories`) whether induction is due
/// before attempting it, riding dreaming's own scheduled cadence rather than
/// a dedicated job/scheduler.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SchemaInductionSettings {
    pub enabled: bool,
    /// Minimum similar supporting memories required before generalizing.
    pub min_examples: u32,
    /// Minimum days since the last induced procedure before trying again.
    pub interval_days: u32,
}

impl Default for SchemaInductionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_examples: 3,
            interval_days: 7,
        }
    }
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
            w_reinforcement: kyma_memory::W_REINFORCEMENT,
            reinforcement: ReinforcementSettings::default(),
            precedent: PrecedentSettings::default(),
            mmr: MmrSettings::default(),
            validity_gate: ValidityGateSettings::default(),
            schema_induction: SchemaInductionSettings::default(),
            dreaming: DreamingSettings::default(),
            hitl: super::memory_policy::HitlPolicy::default(),
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
    fn legacy_settings_row_without_reinforcement_loads_default_off() {
        // A settings row written before M8.1 shipped must still load, with
        // the reinforcement blend defaulting off (zero behavior change).
        let legacy = serde_json::json!({
            "extraction_enabled": true,
            "min_events": 1
        });
        let s: MemorySettings = serde_json::from_value(legacy).unwrap();
        assert!(
            !s.reinforcement.enabled,
            "reinforcement must default off for legacy rows"
        );
        assert_eq!(s.w_reinforcement, 0.0);
    }

    #[test]
    fn reinforcement_roundtrips_through_settings_json() {
        let mut s = MemorySettings::default();
        s.reinforcement.enabled = true;
        s.reinforcement.hit_weight = 0.5;
        s.w_reinforcement = 0.2;
        let v = serde_json::to_value(&s).unwrap();
        let back: MemorySettings = serde_json::from_value(v).unwrap();
        assert!(back.reinforcement.enabled);
        assert_eq!(back.reinforcement.hit_weight, 0.5);
        assert_eq!(back.w_reinforcement, 0.2);
    }

    #[test]
    fn legacy_settings_row_without_precedent_loads_default_on() {
        // Precedent surfacing is purely additive (new response field, no
        // ranking change), so unlike reinforcement it defaults on even for a
        // settings row written before M8.2 shipped.
        let legacy = serde_json::json!({
            "extraction_enabled": true,
            "min_events": 1
        });
        let s: MemorySettings = serde_json::from_value(legacy).unwrap();
        assert!(s.precedent.enabled);
        assert_eq!(
            s.precedent.max_distance,
            kyma_memory::PRECEDENT_MAX_DISTANCE
        );
    }

    #[test]
    fn precedent_roundtrips_through_settings_json() {
        let mut s = MemorySettings::default();
        s.precedent.enabled = false;
        s.precedent.max_distance = 0.05;
        let v = serde_json::to_value(&s).unwrap();
        let back: MemorySettings = serde_json::from_value(v).unwrap();
        assert!(!back.precedent.enabled);
        assert_eq!(back.precedent.max_distance, 0.05);
    }

    #[test]
    fn legacy_settings_row_without_mmr_or_validity_gate_loads_default_off() {
        // Both are ranking/behavior changes, so — unlike precedent — a
        // settings row written before M8.3 shipped must load with both off.
        let legacy = serde_json::json!({
            "extraction_enabled": true,
            "min_events": 1
        });
        let s: MemorySettings = serde_json::from_value(legacy).unwrap();
        assert!(!s.mmr.enabled);
        assert!(!s.validity_gate.enabled);
        assert!(!s.validity_gate.llm_escalation_enabled);
    }

    #[test]
    fn mmr_roundtrips_through_settings_json() {
        let mut s = MemorySettings::default();
        s.mmr.enabled = true;
        s.mmr.lambda = 0.5;
        s.mmr.pool_multiplier = 5;
        let v = serde_json::to_value(&s).unwrap();
        let back: MemorySettings = serde_json::from_value(v).unwrap();
        assert!(back.mmr.enabled);
        assert_eq!(back.mmr.lambda, 0.5);
        assert_eq!(back.mmr.pool_multiplier, 5);
    }

    #[test]
    fn validity_gate_roundtrips_through_settings_json() {
        let mut s = MemorySettings::default();
        s.validity_gate.enabled = true;
        s.validity_gate.llm_escalation_enabled = true;
        s.validity_gate.min_confidence = 0.5;
        let v = serde_json::to_value(&s).unwrap();
        let back: MemorySettings = serde_json::from_value(v).unwrap();
        assert!(back.validity_gate.enabled);
        assert!(back.validity_gate.llm_escalation_enabled);
        assert_eq!(back.validity_gate.min_confidence, 0.5);
    }

    #[test]
    fn legacy_settings_row_without_schema_induction_loads_default_off() {
        let legacy = serde_json::json!({
            "extraction_enabled": true,
            "min_events": 1
        });
        let s: MemorySettings = serde_json::from_value(legacy).unwrap();
        assert!(!s.schema_induction.enabled);
        assert_eq!(s.schema_induction.min_examples, 3);
        assert_eq!(s.schema_induction.interval_days, 7);
    }

    #[test]
    fn schema_induction_roundtrips_through_settings_json() {
        let mut s = MemorySettings::default();
        s.schema_induction.enabled = true;
        s.schema_induction.min_examples = 5;
        s.schema_induction.interval_days = 14;
        let v = serde_json::to_value(&s).unwrap();
        let back: MemorySettings = serde_json::from_value(v).unwrap();
        assert!(back.schema_induction.enabled);
        assert_eq!(back.schema_induction.min_examples, 5);
        assert_eq!(back.schema_induction.interval_days, 14);
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
