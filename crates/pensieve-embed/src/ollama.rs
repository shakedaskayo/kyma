use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Ollama HTTP embedding backend. Calls `POST {base_url}/api/embeddings`
/// with one request per input text (Ollama's embedding endpoint does not
/// natively batch). Dimension must be provided at construction because
/// Ollama's metadata endpoint does not reliably expose it for every model.
#[derive(Debug)]
pub struct OllamaBackend {
    id: String,
    dimension: u16,
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaBackend {
    pub fn new(model: &str, base_url: &str, dimension: u16) -> Result<Self, EmbedError> {
        Ok(Self {
            id: format!("ollama/{model}"),
            dimension,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| EmbedError::Internal(e.to_string()))?,
        })
    }
}

#[derive(Serialize)]
struct Req<'a> {
    model: &'a str,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct Resp {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingBackend for OllamaBackend {
    fn id(&self) -> &str {
        &self.id
    }
    fn dimension(&self) -> u16 {
        self.dimension
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let url = format!("{}/api/embeddings", self.base_url);
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            let resp: Resp = self
                .client
                .post(&url)
                .json(&Req {
                    model: &self.model,
                    prompt: t,
                })
                .send()
                .await
                .map_err(|e| EmbedError::Request(e.to_string()))?
                .error_for_status()
                .map_err(|e| EmbedError::Request(e.to_string()))?
                .json()
                .await
                .map_err(|e| EmbedError::Request(e.to_string()))?;
            if resp.embedding.len() != self.dimension as usize {
                return Err(EmbedError::DimensionMismatch {
                    got: resp.embedding.len() as u16,
                    expected: self.dimension,
                });
            }
            out.push(resp.embedding);
        }
        Ok(out)
    }
}
