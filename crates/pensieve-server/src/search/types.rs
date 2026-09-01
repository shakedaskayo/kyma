//! Unified search envelope: one response shape across data/memory/graph modes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which retrieval backend a unified search dispatches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Data,
    Memory,
    Graph,
}

/// One hit in the unified envelope. Field presence varies by mode; absent fields
/// are omitted from JSON so the `data`-mode shape stays backward-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedHit {
    pub score: f64,
    /// "db.table" (data), realm (memory), or "db/graph" (graph).
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>, // "row" | "memory" | "node"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedSearchResponse {
    pub hits: Vec<UnifiedHit>,
    pub sources_searched: usize,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked: Option<Vec<Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_defaults_to_data_and_parses() {
        #[derive(Deserialize)]
        struct B { #[serde(default)] mode: SearchMode }
        assert_eq!(serde_json::from_str::<B>("{}").unwrap().mode, SearchMode::Data);
        assert_eq!(serde_json::from_str::<B>(r#"{"mode":"memory"}"#).unwrap().mode, SearchMode::Memory);
        assert_eq!(serde_json::from_str::<B>(r#"{"mode":"graph"}"#).unwrap().mode, SearchMode::Graph);
        assert!(serde_json::from_str::<B>(r#"{"mode":"bogus"}"#).is_err());
    }
    #[test]
    fn data_response_omits_mode_context_linked() {
        let r = UnifiedSearchResponse { hits: vec![], sources_searched: 0, elapsed_ms: 0, mode: None, context: None, linked: None };
        let j = serde_json::to_string(&r).unwrap();
        assert!(!j.contains("mode") && !j.contains("context") && !j.contains("linked"), "legacy shape: {j}");
    }
}
