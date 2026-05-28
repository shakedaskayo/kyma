//! Agent engine registry — picks an LLM provider based on persisted config.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineKind {
    Anthropic,
    Openai,
    Ollama,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Ollama => "ollama",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineConfig {
    pub kind: EngineKind,
    pub model: String,
    pub credential_id: Option<Uuid>,
    pub host: Option<String>,
    #[serde(default)]
    pub extras: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_kind_roundtrips_via_json() {
        for kind in [EngineKind::Anthropic, EngineKind::Openai, EngineKind::Ollama] {
            let s = serde_json::to_string(&kind).unwrap();
            let back: EngineKind = serde_json::from_str(&s).unwrap();
            assert_eq!(kind, back, "roundtrip for {kind:?}");
        }
    }

    #[test]
    fn engine_kind_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&EngineKind::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(serde_json::to_string(&EngineKind::Openai).unwrap(), "\"openai\"");
    }
}
