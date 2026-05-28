//! OpenAI engine — adk-rust's OpenAIResponsesClient.

use adk_rust::model::openai::{OpenAIResponsesClient, OpenAIResponsesConfig};
use adk_rust::Llm;
use std::sync::Arc;

use super::{EngineConfig, ResolvedKey};

pub const DEFAULT_MODEL: &str = "gpt-5";

pub fn build(cfg: &EngineConfig, key: ResolvedKey) -> anyhow::Result<Arc<dyn Llm>> {
    let api_key = match key {
        ResolvedKey::ApiKey(k) => k,
        ResolvedKey::None => anyhow::bail!("OpenAI engine requires an API key"),
    };
    let mut llm_cfg = OpenAIResponsesConfig::new(api_key, cfg.model.clone());
    if let Some(host) = &cfg.host {
        llm_cfg = llm_cfg.with_base_url(host);
    }
    let llm = OpenAIResponsesClient::new(llm_cfg)
        .map_err(|e| anyhow::anyhow!("openai init failed: {e:?}"))?;
    Ok(Arc::new(llm))
}

pub fn default_models() -> Vec<&'static str> {
    vec!["gpt-5", "gpt-5-mini", "gpt-4.1", "o4-mini"]
}
