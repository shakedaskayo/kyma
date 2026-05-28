//! Ollama engine — existing default. No API key needed; host is the config.

use adk_rust::model::ollama::{OllamaConfig, OllamaModel};
use adk_rust::Llm;
use std::sync::Arc;

use super::{EngineConfig, ResolvedKey};

pub const DEFAULT_MODEL: &str = "gemma4:latest";
pub const DEFAULT_HOST: &str = "http://localhost:11434";

pub fn build(cfg: &EngineConfig, _key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    let host = cfg
        .host
        .clone()
        .unwrap_or_else(|| DEFAULT_HOST.to_string());
    let llm_cfg = OllamaConfig {
        host,
        model: cfg.model.clone(),
        temperature: Some(0.0),
        num_ctx: None,
        top_p: None,
        top_k: None,
    };
    let llm = OllamaModel::new(llm_cfg)
        .map_err(|e| anyhow::anyhow!("ollama init failed: {e:?}"))?;
    Ok(Arc::new(llm))
}

pub fn default_models() -> Vec<&'static str> {
    vec!["gemma4:latest", "llama4:latest", "qwen3:latest", "mistral:latest"]
}
