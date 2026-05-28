//! Anthropic engine — adk-rust's AnthropicClient.

use adk_rust::model::anthropic::{AnthropicClient, AnthropicConfig};
use adk_rust::Llm;
use std::sync::Arc;

use super::{EngineConfig, ResolvedKey};

pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

pub fn build(cfg: &EngineConfig, key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    let api_key = match key {
        ResolvedKey::ApiKey(k) => k,
        ResolvedKey::None => anyhow::bail!("Anthropic engine requires an API key"),
    };
    let mut llm_cfg = AnthropicConfig::new(api_key, cfg.model.clone());
    if let Some(host) = &cfg.host {
        llm_cfg = llm_cfg.with_base_url(host);
    }
    if let Some(mt) = cfg.extras.get("max_tokens").and_then(|v| v.as_u64()) {
        // Clamp instead of `as u32` truncating — `with_max_tokens` is u32, the
        // upstream cap is far below u32::MAX anyway, but silent truncation on a
        // user-supplied number is a bad default.
        llm_cfg = llm_cfg.with_max_tokens(mt.try_into().unwrap_or(u32::MAX));
    }
    let llm = AnthropicClient::new(llm_cfg)
        .map_err(|e| anyhow::anyhow!("anthropic init failed: {e:?}"))?;
    Ok(Arc::new(llm))
}

pub fn default_models() -> Vec<String> {
    vec![
        "claude-opus-4-7".into(),
        "claude-sonnet-4-6".into(),
        "claude-haiku-4-5".into(),
    ]
}
