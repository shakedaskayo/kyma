use crate::{EmbedError, EmbeddingBackend};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Google Gemini native embeddings endpoint (`:embedContent`). One request
/// per input text — Gemini's single-shot embedContent endpoint doesn't
/// batch; the batchEmbedContents variant has a separate shape and is out
/// of scope for Slice 1.
#[derive(Debug)]
pub struct GeminiBackend {
    id: String,
    dimension: u16,
    model: String,
    api_key: String,
    client: reqwest::Client,
}

impl GeminiBackend {
    pub fn new(model: &str, dimension: u16, api_key: String) -> Result<Self, EmbedError> {
        Ok(Self {
            id: format!("gemini/{model}"),
            dimension,
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
struct Part<'a> {
    text: &'a str,
}
#[derive(Serialize)]
struct Content<'a> {
    parts: Vec<Part<'a>>,
}
#[derive(Serialize)]
struct Req<'a> {
    model: String,
    content: Content<'a>,
}
#[derive(Deserialize)]
struct Emb {
    values: Vec<f32>,
}
#[derive(Deserialize)]
struct Resp {
    embedding: Emb,
}

#[async_trait]
impl EmbeddingBackend for GeminiBackend {
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
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:embedContent?key={}",
            self.model, self.api_key
        );
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            let resp: Resp = self
                .client
                .post(&url)
                .json(&Req {
                    model: format!("models/{}", self.model),
                    content: Content {
                        parts: vec![Part { text: t }],
                    },
                })
                .send()
                .await
                .map_err(|e| EmbedError::Request(e.to_string()))?
                .error_for_status()
                .map_err(|e| EmbedError::Request(e.to_string()))?
                .json()
                .await
                .map_err(|e| EmbedError::Request(e.to_string()))?;
            if resp.embedding.values.len() != self.dimension as usize {
                return Err(EmbedError::DimensionMismatch {
                    got: resp.embedding.values.len() as u16,
                    expected: self.dimension,
                });
            }
            out.push(resp.embedding.values);
        }
        Ok(out)
    }
}
