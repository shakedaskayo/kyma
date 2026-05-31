//! Memory domain types.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The kind of a memory. Mirrors agentcy's taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Fact,
    Decision,
    Preference,
    Learning,
    Summary,
}

impl MemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::Fact => "fact",
            MemoryType::Decision => "decision",
            MemoryType::Preference => "preference",
            MemoryType::Learning => "learning",
            MemoryType::Summary => "summary",
        }
    }

    /// Parse loosely; unrecognized values fall back to `Fact`.
    pub fn parse(s: &str) -> MemoryType {
        match s.trim().to_ascii_lowercase().as_str() {
            "decision" => MemoryType::Decision,
            "preference" => MemoryType::Preference,
            "learning" => MemoryType::Learning,
            "summary" => MemoryType::Summary,
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

/// Input to [`crate::MemoryWriter::save`].
#[derive(Debug, Clone)]
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
        }
    }
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
}
