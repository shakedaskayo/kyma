//! Agent engine registry — picks an LLM provider based on persisted config.

use adk_rust::Llm;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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

pub mod anthropic;
pub mod claude_creds;
pub mod ollama;
pub mod openai;
pub mod resolver;
pub mod store;
pub use resolver::{CredentialResolver, ResolvedKey};
pub use store::{EnginePreferenceStore, PgEnginePreferenceStore};

/// Construct an `Llm` for the given engine config + resolved credential.
pub fn build_engine(cfg: &EngineConfig, key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    match cfg.kind {
        EngineKind::Anthropic => anthropic::build(cfg, key),
        EngineKind::Openai => openai::build(cfg, key),
        EngineKind::Ollama => ollama::build(cfg, key),
    }
}

/// Available providers and their default model menus. Returned by
/// `GET /v1/agent/engines` so the UI can render the picker.
#[derive(Debug, serde::Serialize)]
pub struct EngineSummary {
    pub kind: EngineKind,
    pub label: &'static str,
    pub models: Vec<&'static str>,
    pub needs_key: bool,
}

pub fn engine_catalogue() -> Vec<EngineSummary> {
    vec![
        EngineSummary {
            kind: EngineKind::Anthropic,
            label: "Anthropic (Claude)",
            models: anthropic::default_models(),
            needs_key: true,
        },
        EngineSummary {
            kind: EngineKind::Openai,
            label: "OpenAI",
            models: openai::default_models(),
            needs_key: true,
        },
        EngineSummary {
            kind: EngineKind::Ollama,
            label: "Ollama (local)",
            models: ollama::default_models(),
            needs_key: false,
        },
    ]
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
