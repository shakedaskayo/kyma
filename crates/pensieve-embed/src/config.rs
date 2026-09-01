use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "provider")]
pub enum EmbeddingProvider {
    Fastembed {
        id: String,
        model_path: Option<String>,
    },
    Ollama {
        id: String,
        base_url: String,
    },
    #[serde(rename = "openai-compat")]
    OpenAICompat {
        id: String,
        base_url: String,
        api_key_env: Option<String>,
    },
    Gemini {
        id: String,
        api_key_env: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,
}

impl EmbeddingConfig {
    /// Defaults to fastembed bge-small-en-v1.5 (384-dim).
    pub fn default_fastembed() -> Self {
        Self {
            provider: EmbeddingProvider::Fastembed {
                id: "bge-small-en-v1.5".into(),
                model_path: None,
            },
        }
    }

    /// Load from env vars: `PENSIEVE_EMBED_PROVIDER`, `PENSIEVE_EMBED_MODEL_ID`,
    /// `PENSIEVE_EMBED_BASE_URL`, `PENSIEVE_EMBED_MODEL_PATH`. Returns the default
    /// fastembed config when none are set.
    pub fn from_env() -> Self {
        let provider = std::env::var("PENSIEVE_EMBED_PROVIDER").ok();
        let id = std::env::var("PENSIEVE_EMBED_MODEL_ID").ok();
        match provider.as_deref() {
            Some("ollama") => Self {
                provider: EmbeddingProvider::Ollama {
                    id: id.unwrap_or_else(|| "nomic-embed-text".into()),
                    base_url: std::env::var("PENSIEVE_EMBED_BASE_URL")
                        .unwrap_or_else(|_| "http://localhost:11434".into()),
                },
            },
            Some("openai-compat") => Self {
                provider: EmbeddingProvider::OpenAICompat {
                    id: id.unwrap_or_else(|| "text-embedding-3-small".into()),
                    base_url: std::env::var("PENSIEVE_EMBED_BASE_URL")
                        .unwrap_or_else(|_| "https://api.openai.com/v1".into()),
                    api_key_env: Some(
                        std::env::var("PENSIEVE_EMBED_API_KEY_ENV")
                            .unwrap_or_else(|_| "OPENAI_API_KEY".into()),
                    ),
                },
            },
            Some("gemini") => Self {
                provider: EmbeddingProvider::Gemini {
                    id: id.unwrap_or_else(|| "text-embedding-004".into()),
                    api_key_env: "GOOGLE_API_KEY".into(),
                },
            },
            Some("fastembed") => Self {
                provider: EmbeddingProvider::Fastembed {
                    id: id.unwrap_or_else(|| "bge-small-en-v1.5".into()),
                    model_path: std::env::var("PENSIEVE_EMBED_MODEL_PATH").ok(),
                },
            },
            _ => Self::default_fastembed(),
        }
    }
}
