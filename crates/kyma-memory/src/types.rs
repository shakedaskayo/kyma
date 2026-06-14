//! Memory domain types.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// The kind of a memory. Mirrors agentcy's taxonomy.
///
/// `Procedure` captures "how things are done" (conventions, runbooks) and
/// `Entity` is a lightweight, embeddable node minted for an extracted entity
/// that is then linked (`RESOLVES_TO`) to the real catalog graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Fact,
    Decision,
    Preference,
    Learning,
    Summary,
    Procedure,
    Entity,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::Fact => "fact",
            MemoryType::Decision => "decision",
            MemoryType::Preference => "preference",
            MemoryType::Learning => "learning",
            MemoryType::Summary => "summary",
            MemoryType::Procedure => "procedure",
            MemoryType::Entity => "entity",
        }
    }

    /// Parse loosely; unrecognized values fall back to `Fact`.
    pub fn parse(s: &str) -> MemoryType {
        match s.trim().to_ascii_lowercase().as_str() {
            "decision" => MemoryType::Decision,
            "preference" => MemoryType::Preference,
            "learning" => MemoryType::Learning,
            "summary" => MemoryType::Summary,
            "procedure" => MemoryType::Procedure,
            "entity" => MemoryType::Entity,
            _ => MemoryType::Fact,
        }
    }
}

impl Default for MemoryType {
    fn default() -> Self {
        MemoryType::Fact
    }
}

/// Lifecycle status. `archived` memories are hidden from recall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Background,
    Archived,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Active => "active",
            MemoryStatus::Background => "background",
            MemoryStatus::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<MemoryStatus> {
        match s.trim().to_ascii_lowercase().as_str() {
            "active" => Some(MemoryStatus::Active),
            "background" => Some(MemoryStatus::Background),
            "archived" => Some(MemoryStatus::Archived),
            _ => None,
        }
    }
}

/// Memory class (agentcy / HippoRAG taxonomy) governing retention and
/// consolidation, orthogonal to [`MemoryType`] (the content kind):
/// - `Episodic` — time-stamped events; decays with age (finite half-life) so
///   stale episodes fade unless reinforced.
/// - `Semantic` — distilled, durable facts; no age decay, superseded only by
///   bi-temporal invalidation.
/// - `Procedural` — how-to knowledge; demoted by repeated failure, not by age.
///
/// A memory may start `Episodic` when captured and become `Semantic` once the
/// dreaming consolidation stage distills it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryClass {
    Episodic,
    Semantic,
    Procedural,
}

impl MemoryClass {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryClass::Episodic => "episodic",
            MemoryClass::Semantic => "semantic",
            MemoryClass::Procedural => "procedural",
        }
    }

    /// Parse loosely; unrecognized → `None` (the class is optional metadata).
    pub fn parse(s: &str) -> Option<MemoryClass> {
        match s.trim().to_ascii_lowercase().as_str() {
            "episodic" => Some(MemoryClass::Episodic),
            "semantic" => Some(MemoryClass::Semantic),
            "procedural" => Some(MemoryClass::Procedural),
            _ => None,
        }
    }

    /// Default class for a [`MemoryType`] when a writer doesn't specify one.
    /// The memory engine stores *distilled* memories, so most types are durable
    /// `Semantic` knowledge; `Procedure` is `Procedural` (how-to, demoted by
    /// failure) and `Summary` is `Episodic` (a time-bound recap that should fade
    /// unless reinforced). Dreaming may later re-classify (e.g. consolidate an
    /// episodic recap into a semantic fact).
    pub fn default_for(memory_type: MemoryType) -> MemoryClass {
        match memory_type {
            MemoryType::Procedure => MemoryClass::Procedural,
            MemoryType::Summary => MemoryClass::Episodic,
            MemoryType::Fact
            | MemoryType::Decision
            | MemoryType::Preference
            | MemoryType::Learning
            | MemoryType::Entity => MemoryClass::Semantic,
        }
    }

    /// Age-decay half-life in days, or `None` for classes that do not decay with
    /// age (semantic = invalidation-only; procedural = failure-driven).
    pub fn half_life_days(self) -> Option<f64> {
        match self {
            MemoryClass::Episodic => Some(7.0),
            MemoryClass::Semantic | MemoryClass::Procedural => None,
        }
    }

    /// Recency-decay multiplier in `[0, 1]` for a memory `age_days` old:
    /// exponential (halving once per half-life) for decaying classes, `1.0` for
    /// non-decaying ones. A scoring boost, never an eviction trigger.
    pub fn decay_weight(self, age_days: f64) -> f64 {
        match self.half_life_days() {
            Some(hl) if hl > 0.0 => 0.5_f64.powf(age_days.max(0.0) / hl),
            _ => 1.0,
        }
    }
}

/// Input to [`crate::MemoryWriter::save`].
///
/// Serializable so queued saves can be persisted to the durable job store
/// (`kyma-queue`) and replayed after a crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemory {
    pub content: String,
    pub title: Option<String>,
    pub memory_type: MemoryType,
    pub tags: Vec<String>,
    pub realm: String,
    pub importance: f32,
    /// Existing graph node ids this memory is about (become `REFERENCES` edges).
    pub references: Vec<String>,
    pub source_session_id: Option<Uuid>,
    pub source_run_id: Option<Uuid>,
    /// When the fact became true (bi-temporal). Defaults to `created_at`.
    pub valid_at: Option<String>,
    /// How this memory was formed: `{source, run_id, watermark, extracted_from_ids, confidence}`.
    pub provenance: Option<Value>,
    /// Deterministic upsert key (e.g. `architecture/auth-model`). When set, a
    /// save with the same `(realm, topic_key)` updates the existing memory in
    /// place instead of creating a new one. `None` = always create.
    pub topic_key: Option<String>,
    /// Retention/consolidation class (see [`MemoryClass`]). `None` until a
    /// writer or the dreaming consolidation stage assigns one.
    pub memory_class: Option<MemoryClass>,
    /// Visibility scope: `None`/`"public"` is shared; `"private:<agent>"` is
    /// readable only by that agent. Enforced in recall (later increment).
    pub space: Option<String>,
    /// The agent that wrote this memory (provenance for space scoping and
    /// per-agent attribution). `None` for un-attributed/manual writes.
    pub writer_agent_id: Option<String>,
}

impl CreateMemory {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            title: None,
            memory_type: MemoryType::Fact,
            tags: Vec::new(),
            realm: crate::DEFAULT_REALM.to_string(),
            importance: 0.5,
            references: Vec::new(),
            source_session_id: None,
            source_run_id: None,
            valid_at: None,
            provenance: None,
            topic_key: None,
            memory_class: None,
            space: None,
            writer_agent_id: None,
        }
    }
}

/// One row of the memory provenance summary: how many latest-version,
/// non-archived memories were formed by `source` in `realm`. `source` is
/// `provenance.source` (e.g. "claude-code", "dreaming"); memories without a
/// provenance source are reported as `"manual"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSummary {
    pub source: String,
    pub realm: String,
    pub count: i64,
}

/// Filters for recall / listing.
#[derive(Debug, Clone, Default)]
pub struct RecallFilter {
    /// Realms to include (the project ∪ global set). Empty = all realms.
    pub realms: Vec<String>,
    pub memory_type: Option<MemoryType>,
    /// Statuses to include. Empty = active + background (archived always excluded).
    pub statuses: Vec<MemoryStatus>,
    pub tags: Vec<String>,
    pub importance_min: Option<f32>,
    pub since: Option<String>,
    pub until: Option<String>,
    /// Point-in-time recall: only memories valid at this instant
    /// (`valid_at <= as_of < invalid_at`). `None` = "now".
    pub as_of: Option<String>,
    /// Include superseded/invalidated memories (default false). Used for audit.
    pub include_invalidated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_class_roundtrips_and_parses_loosely() {
        for c in [
            MemoryClass::Episodic,
            MemoryClass::Semantic,
            MemoryClass::Procedural,
        ] {
            assert_eq!(MemoryClass::parse(c.as_str()), Some(c));
        }
        assert_eq!(MemoryClass::parse("  Episodic "), Some(MemoryClass::Episodic));
        assert_eq!(MemoryClass::parse("nonsense"), None);
    }

    #[test]
    fn only_episodic_decays_with_age() {
        // Episodic halves every 7 days; others never decay with age.
        let ep = MemoryClass::Episodic;
        assert!((ep.decay_weight(0.0) - 1.0).abs() < 1e-9);
        assert!((ep.decay_weight(7.0) - 0.5).abs() < 1e-9);
        assert!((ep.decay_weight(14.0) - 0.25).abs() < 1e-9);
        assert_eq!(MemoryClass::Semantic.half_life_days(), None);
        assert_eq!(MemoryClass::Procedural.half_life_days(), None);
        assert_eq!(MemoryClass::Semantic.decay_weight(365.0), 1.0);
        assert_eq!(MemoryClass::Procedural.decay_weight(365.0), 1.0);
    }

    #[test]
    fn create_memory_class_fields_default_none() {
        let m = CreateMemory::new("hi");
        assert!(m.memory_class.is_none());
        assert!(m.space.is_none());
        assert!(m.writer_agent_id.is_none());
    }
}
