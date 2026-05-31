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

pub fn default_models() -> Vec<String> {
    vec![
        "gemma4:latest".into(),
        "llama4:latest".into(),
        "qwen3:latest".into(),
        "mistral:latest".into(),
    ]
}

/// Live-fetch the models actually installed in the user's Ollama instance.
/// Falls back to [`default_models`] if the host is unreachable or returns
/// garbage — the picker still shows *something* useful in that case.
pub async fn installed_models(host: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Tag {
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        models: Vec<Tag>,
    }

    let url = format!("{}/api/tags", host.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(_) => return default_models(),
    };
    let Ok(resp) = client.get(url).send().await else {
        return default_models();
    };
    let Ok(parsed) = resp.json::<TagsResponse>().await else {
        return default_models();
    };
    let mut names: Vec<String> = parsed.models.into_iter().map(|t| t.name).collect();
    if names.is_empty() {
        return default_models();
    }
    names.sort();
    names
}
