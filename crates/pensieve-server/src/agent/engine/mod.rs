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
    /// Spawn the local `claude` CLI as the engine — inherits Keychain OAuth,
    /// MCPs, skills, and everything else Claude Code has configured on the
    /// host. The /v1/agent/ask handler bypasses adk-rust for this kind.
    ClaudeCli,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Ollama => "ollama",
            Self::ClaudeCli => "claude_cli",
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
pub mod claude_cli;
pub mod claude_creds;
pub mod ollama;
pub mod openai;
pub mod resolver;
pub mod store;
pub use resolver::{CredentialResolver, ResolvedKey};
pub use store::{EnginePreferenceStore, PgEnginePreferenceStore};

/// Construct an `Llm` for the given engine config + resolved credential.
///
/// The Claude CLI engine bypasses the adk-rust path entirely (its own tool
/// loop is what makes it useful), so this returns a typed error for that
/// kind — callers must check `cfg.kind == ClaudeCli` first and route
/// through `claude_cli::run` instead.
pub fn build_engine(cfg: &EngineConfig, key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    match cfg.kind {
        EngineKind::Anthropic => anthropic::build(cfg, key),
        EngineKind::Openai => openai::build(cfg, key),
        EngineKind::Ollama => ollama::build(cfg, key),
        EngineKind::ClaudeCli => anyhow::bail!(
            "the claude_cli engine doesn't run through adk-rust — call claude_cli::run directly"
        ),
    }
}

/// Available providers and their default model menus. Returned by
/// `GET /v1/agent/engines` so the UI can render the picker.
#[derive(Debug, serde::Serialize)]
pub struct EngineSummary {
    pub kind: EngineKind,
    pub label: &'static str,
    pub models: Vec<String>,
    pub needs_key: bool,
}

/// Build the engine catalogue, live-fetching the Ollama model list from the
/// active config's host (so the picker shows actually-installed models, not
/// phantom defaults like `gemma4:latest` that nobody pulled).
///
/// `ollama_host_hint` is consulted ONLY for the live `/api/tags` fetch — the
/// other providers' lists are static so they're returned regardless.
pub async fn engine_catalogue(ollama_host_hint: Option<&str>) -> Vec<EngineSummary> {
    let host = ollama_host_hint.unwrap_or(ollama::DEFAULT_HOST);
    let ollama_models = ollama::installed_models(host).await;
    let mut out = vec![
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
            models: ollama_models,
            needs_key: false,
        },
    ];
    // Only advertise the Claude CLI engine if `claude` is actually on
    // PATH — picking it on a host without Claude Code installed would just
    // error every ask.
    if claude_cli::locate_binary().is_some() {
        out.push(EngineSummary {
            kind: EngineKind::ClaudeCli,
            label: "Claude Code (local CLI)",
            models: claude_cli::default_models(),
            needs_key: false,
        });
    }
    out
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
