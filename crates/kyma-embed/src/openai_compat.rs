use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// OpenAI-compatible embeddings endpoint. Works with OpenAI, Together,
/// Fireworks, Mistral, local llama.cpp servers, etc. — anything that
/// implements `POST {base_url}/embeddings` with OpenAI's request/response
/// shape. A single request batches the whole `texts` slice, which is the
/// main reason to prefer this backend over Ollama for cloud-style use.
#[derive(Debug)]
pub struct OpenAICompatBackend {
    id: String,
    dimension: u16,
    base_url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
}

impl OpenAICompatBackend {
    pub fn new(
        model: &str,
        base_url: &str,
        dimension: u16,
        api_key: Option<String>,
    ) -> Result<Self, EmbedError> {
        Ok(Self {
            id: format!("openai-compat/{model}"),
            dimension,
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            api_key,
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
    input: &'a [String],
}

#[derive(Deserialize)]
struct Item {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct Resp {
    data: Vec<Item>,
}

#[async_trait]
impl EmbeddingBackend for OpenAICompatBackend {
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
        let url = format!("{}/embeddings", self.base_url);
        let mut req = self.client.post(&url).json(&Req {
            model: &self.model,
            input: texts,
        });
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }
        let resp: Resp = req
            .send()
            .await
            .map_err(|e| EmbedError::Request(e.to_string()))?
            .error_for_status()
            .map_err(|e| EmbedError::Request(e.to_string()))?
            .json()
            .await
            .map_err(|e| EmbedError::Request(e.to_string()))?;
        let out: Vec<_> = resp.data.into_iter().map(|i| i.embedding).collect();
        if out.len() != texts.len() {
            return Err(EmbedError::Internal(format!(
                "openai-compat: expected {} embeddings, got {}",
                texts.len(),
                out.len()
            )));
        }
        if let Some(v) = out.first() {
            if v.len() != self.dimension as usize {
                return Err(EmbedError::DimensionMismatch {
                    got: v.len() as u16,
                    expected: self.dimension,
                });
            }
        }
        Ok(out)
    }
}
